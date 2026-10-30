//! One viewer: its WebSocket and its own connection to the session's display.
//!
//! Client messages are decoded and checked before anything is forwarded; input goes on only from the lease holder at
//! the current epoch, and what the person types is recorded for redaction. Display messages go out through the
//! session's network simulator. Every queue and frame is bounded, and a client that stops answering pings loses its
//! stream (and the lease with it).
//!
//! Downloads: each one offered to the holder comes with a single-use ticket (`FileLink`) for the download endpoint.
//! When the person's browser fetches it, this relay receives its bytes from the display in place of the WebSocket,
//! granting credit (`FileRead`) only as the response takes them, so at most [`FILE_WINDOW`] bytes wait here, and never
//! waiting on the response: a piece that does not fit fails the download.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message as WsMessage, WebSocket};
use display_protocol::frame::{FrameReader, Link, MAX_CHUNK, PAGE_OPEN_WAIT};
use display_protocol::{Direction, FILE_TICKET, FILE_WINDOW, KeyKind, LeaseHolder, Message};
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{Notify, mpsc, watch};

use crate::app::{
    App, Diversion, DownloadProgress, Features, Network, Piece, Session, Ticket, ToViewer, Viewer, ViewerStats,
};
use crate::auth::LoginKey;
use crate::redact::{Edit, Redactions, evdev_char};
use crate::udp::{Path, Udp};

const PING_EVERY: Duration = Duration::from_secs(10);
const PONG_DEADLINE: Duration = Duration::from_secs(20);
/// How long a stopped session's relay may take to close on its own before it is cut.
const CLOSE_GRACE: Duration = Duration::from_secs(2);
const TO_VIEWER_QUEUE: usize = 64;
/// Display messages, and their bytes, waiting for the simulated network before the relay stops reading from the
/// display: messages of up to 2 MiB would otherwise let 256 of them hold 512 MiB per viewer. The byte bound is a few
/// MiB because every viewer has one and the admin's whole memory limit is 256 MiB. A read is admitted under both
/// bounds, so they are overshot by at most what one read decodes.
const OUTBOX_LIMIT: usize = 256;
const OUTBOX_BYTES: usize = 4 * 1024 * 1024;
/// Download tickets one viewer holds at once; the oldest goes first.
const MAX_TICKETS: usize = 32;
/// At most one direct path's offer per viewer this often, and how often its session is checked for stalled media.
const OFFER_EVERY: Duration = Duration::from_secs(10);
const PATH_CHECK: Duration = Duration::from_millis(250);
/// Credit is granted again once the response took this much more.
const CREDIT_EVERY: u64 = FILE_WINDOW / 4;
use display_protocol::keys::{
    BACKSPACE as KEY_BACKSPACE, DOWN as KEY_DOWN, ENTER as KEY_ENTER, ESC as KEY_ESC, LEFT as KEY_LEFT,
    LEFTALT as KEY_LEFTALT, LEFTCTRL as KEY_LEFTCTRL, LEFTMETA as KEY_LEFTMETA, LEFTSHIFT as KEY_LEFTSHIFT,
    RIGHT as KEY_RIGHT, RIGHTSHIFT as KEY_RIGHTSHIFT, TAB as KEY_TAB, UP as KEY_UP,
};
const KEY_RIGHTCTRL: u16 = 97;
const KEY_RIGHTALT: u16 = 100;
const KEY_RIGHTMETA: u16 = 126;
const KEY_HOME: u16 = 102;
const KEY_PAGEUP: u16 = 104;
const KEY_END: u16 = 107;
const KEY_PAGEDOWN: u16 = 109;
const KEY_INSERT: u16 = 110;
const KEY_DELETE: u16 = 111;
const KEY_KPENTER: u16 = 96;
/// The keypad from 7 to its decimal point: whether one moves the cursor or types depends on the remote Num Lock.
const KEYPAD: std::ops::RangeInclusive<u16> = 71..=83;
/// The modifier keys the typed-run record follows: bit `i` of [`Modifiers`] is `MODIFIERS[i]` held. Shift (the first
/// two) picks a key's character; Ctrl, Alt or Meta held make a key press a shortcut.
const MODIFIERS: [u16; 8] = [
    KEY_LEFTSHIFT,
    KEY_RIGHTSHIFT,
    KEY_LEFTCTRL,
    KEY_RIGHTCTRL,
    KEY_LEFTALT,
    KEY_RIGHTALT,
    KEY_LEFTMETA,
    KEY_RIGHTMETA,
];

/// The modifier keys one viewer holds down, at the lease epoch they were pressed under. The display releases every
/// key at a lease change (grant, give-back, a lost holder, a takeover; each one moves the epoch on), and the release
/// never comes from the viewer: input at a new epoch starts with every key up, here too.
#[derive(Default)]
struct Modifiers(u8, u32);

impl Modifiers {
    /// Releases every key held under another epoch than `epoch`.
    fn at(&mut self, epoch: u32) {
        if self.1 != epoch {
            *self = Modifiers(0, epoch);
        }
    }

    fn bit(code: u16) -> Option<u8> {
        MODIFIERS.iter().position(|&m| m == code).map(|i| 1 << i)
    }

    fn shift(&self) -> bool {
        self.0 & 0b11 != 0
    }

    fn chord(&self) -> bool {
        self.0 & !0b11 != 0
    }
}

/// `deadline` (a demo visitor's deadline) ends the relay there, whatever it is waiting on: a blocked write to a slow
/// client is cancelled too. So does the session being stopped.
pub async fn run(
    app: Arc<App>,
    session: Arc<Session>,
    login: LoginKey,
    token: String,
    socket: WebSocket,
    deadline: Option<std::time::Instant>,
) {
    let viewer = app.next_viewer.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let path = app.config.streams_dir.join(&session.id).join("display.sock");
    let display = match UnixStream::connect(&path).await {
        Ok(stream) => stream,
        Err(e) => {
            tracing::warn!(session = %session.id, %e, "the display stream is unavailable");
            return;
        }
    };
    let (tx, rx) = mpsc::channel(TO_VIEWER_QUEUE);
    // Told the lease as it is admitted.
    let (lease, lease_rx) = watch::channel((0, LeaseHolder::Agent));
    let wake = Arc::new(Notify::new());
    let entry = Viewer { login, token, tx, lease, wake: wake.clone(), stats: ViewerStats::default() };
    session.admit(&app, viewer, entry);
    tracing::info!(session = %session.id, viewer, "viewer connected");
    let mut ended = session.ended.subscribe();
    let relayed = async {
        tokio::select! {
            outcome = relay(&session, app.udp.as_ref(), viewer, login, display, socket, rx, lease_rx, &wake) => outcome,
            // A grace period first, so a relay that can still deliver the session's close does.
            _ = async {
                let _ = ended.wait_for(|ended| *ended).await;
                tokio::time::sleep(CLOSE_GRACE).await;
            } => Err("the session ended".to_owned()),
            // A logout or an expiry closes the login's streams through the relay's queue, where that close may find no
            // room, and the relay may be blocked writing: checked here too, so no stream outlives its login.
            _ = async {
                let mut every = tokio::time::interval(PING_EVERY);
                while app.auth.lock().unwrap().is_live(&login) {
                    every.tick().await;
                }
            } => Err(crate::app::ENDED.to_owned()),
        }
    };
    let outcome = match deadline {
        Some(at) => tokio::time::timeout_at(at.into(), relayed).await.unwrap_or(Err("the demo visit ended".into())),
        None => relayed.await,
    };
    let mut state = session.state.lock().unwrap();
    state.viewers.remove(&viewer);
    if state.viewers.is_empty() {
        state.unwatched_since = Some(Instant::now());
    }
    state.tickets.retain(|_, t| t.viewer != viewer);
    if let Some(udp) = &app.udp {
        udp.forget(&session.id, viewer);
    }
    if state.holder == Some(viewer) {
        session.revoke_lost_holder(&mut state);
        tracing::info!(session = %session.id, viewer, "the control holder left; lease revoked");
    }
    tracing::info!(session = %session.id, viewer, ?outcome, "viewer closed");
}

/// A download this relay feeds to the download endpoint.
struct Diverted {
    taken: Diversion,
    /// Bytes received from the display, and the consumed count last granted as credit.
    received: u64,
    credited: u64,
}

impl Diverted {
    /// Takes the next piece, if it is the next in order, within the credit granted, and fits; false fails it.
    fn take(&mut self, offset: u64, data: Vec<u8>) -> bool {
        let consumed = self.taken.progress.consumed.load(Ordering::Acquire);
        let len = data.len() as u64;
        let past_size = self.taken.size.is_some_and(|size| self.received + len > size);
        if offset != self.received || self.received + len > consumed + FILE_WINDOW || past_size {
            return false;
        }
        if self.taken.pieces.try_send(Piece::Data(data.into())).is_err() {
            return false;
        }
        self.received += len;
        true
    }
}

#[allow(clippy::too_many_arguments)]
async fn relay(
    session: &Session,
    udp: Option<&Udp>,
    viewer: u64,
    login: LoginKey,
    display: UnixStream,
    socket: WebSocket,
    mut from_app: mpsc::Receiver<ToViewer>,
    mut lease: watch::Receiver<(u32, LeaseHolder)>,
    wake: &Notify,
) -> Result<(), String> {
    let (mut display_rx, mut display_tx) = display.into_split();
    send_display(&mut display_tx, &Link::Attach { viewer }).await?;
    let mut settings = session.settings.subscribe();
    let current = *settings.borrow_and_update();
    send_display(&mut display_tx, &Link::Settings(current)).await?;
    let (mut ws_tx, mut ws_rx) = socket.split();
    let mut frames = FrameReader::new(Direction::ServerToClient);
    let mut chunk = vec![0u8; MAX_CHUNK];
    // Each with its lane, and as the display sent it (the link's relay tag first): its path is chosen as it leaves.
    let mut outbox: VecDeque<(Instant, Lane, Bytes)> = VecDeque::new();
    let mut outbox_bytes: usize = 0;
    let mut next_free = Instant::now();
    let mut ping = tokio::time::interval(PING_EVERY);
    let mut last_pong = Instant::now();
    let mut held = Modifiers::default();
    let mut hello = false;
    let mut downloads: HashMap<u32, Diverted> = HashMap::new();
    // Downloads whose end the display confirmed, until their response took it: the person hears the end (FileEnd)
    // only then, so a page that waits for it (a recording before Leave) never tears down bytes still on their way.
    let mut delivering: HashMap<u32, Delivering> = HashMap::new();
    // Pages this viewer's browser was asked to open (a demo switch), awaiting the display's answer.
    let mut opening: HashMap<u32, (Instant, tokio::sync::oneshot::Sender<bool>)> = HashMap::new();
    let mut next_open: u32 = 0;
    let mut tickets: VecDeque<[u8; FILE_TICKET]> = VecDeque::new();
    // The direct path: the session carrying this viewer's media, the offer its ticket must come from, and when the
    // last offer went.
    let mut path: Option<Path> = None;
    let mut generation: u32 = 0;
    let mut offered: Option<u32> = None;
    let mut last_offer: Option<Instant> = None;
    let mut watch = tokio::time::interval(PATH_CHECK);
    loop {
        let due = outbox.front().map(|(t, _, _)| *t);
        tokio::select! {
            read = display_rx.read(&mut chunk), if outbox.len() < OUTBOX_LIMIT && outbox_bytes < OUTBOX_BYTES => {
                let n = read.map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("the display closed".into());
                }
                frames.push(&chunk[..n]).map_err(|e| e.to_string())?;
                // The simulated network, read once per read from the display and only when a message goes through
                // it; a change takes effect from the next read.
                let mut conditions = None;
                while let Some(body) = frames.next_body().map_err(|e| e.to_string())? {
                    let message = match Link::decode(&body, Direction::ServerToClient).map_err(|e| e.to_string())? {
                        Link::Relay(message) => message,
                        Link::PageOpened { request, opened } => {
                            if let Some((_, answer)) = opening.remove(&request) {
                                let _ = answer.send(opened);
                            }
                            continue;
                        }
                        _ => return Err("the display sent link control".into()),
                    };
                    record_server(session, viewer, &message);
                    // A feature that is off: what it carries goes no further, before any download is diverted.
                    if let Some(answer) = withheld(session, viewer, &message) {
                        if let Some(answer) = answer {
                            send_display(&mut display_tx, &answer).await?;
                        }
                        continue;
                    }
                    let message = match message {
                        Message::FileLink { .. } => return Err("the display sent a download ticket".into()),
                        Message::UdpOffer { .. } | Message::MediaMoved { .. } | Message::MediaFallback { .. } => {
                            return Err("the display sent a direct path's message".into());
                        }
                        // A download's bytes go to its response alone, never to the WebSocket.
                        Message::FileData { lease_epoch, file, offset, data } => {
                            let fits = match downloads.get_mut(&file) {
                                Some(d) if d.taken.lease_epoch == lease_epoch && holds(session, viewer, lease_epoch) => {
                                    d.take(offset, data)
                                }
                                _ => continue,
                            };
                            if !fits {
                                fail(&mut downloads, file, &mut display_tx).await?;
                            }
                            continue;
                        }
                        Message::FileEnd { lease_epoch, file, size } => {
                            if let Some(d) = downloads.remove(&file) {
                                let whole = d.taken.lease_epoch == lease_epoch
                                    && d.received == size
                                    && d.taken.size.is_none_or(|offered| offered == size);
                                let _ = d.taken.pieces.try_send(if whole { Piece::End } else { Piece::Fail });
                                if whole {
                                    let end = Bytes::from(body).slice(1..);
                                    delivering.insert(file, Delivering { lease_epoch, end, progress: d.taken.progress });
                                    deliver(session, viewer, &mut delivering, &mut ws_tx).await?;
                                    continue;
                                }
                            }
                            // Not delivered whole (cancelled, cut short, or of another lease): the client hears it is
                            // gone, never that it ended, which would show it saved.
                            if holds(session, viewer, lease_epoch) {
                                let gone = Message::FileGone { lease_epoch, file }.encode();
                                send(&mut ws_tx, WsMessage::Binary(gone.into())).await?;
                            }
                            continue;
                        }
                        Message::FileGone { lease_epoch, file } => {
                            if let Some(d) = downloads.remove(&file) {
                                let _ = d.taken.pieces.try_send(Piece::Fail);
                            }
                            Message::FileGone { lease_epoch, file }
                        }
                        other => other,
                    };
                    // A message that decodes re-encodes to the same bytes (the `message` fuzz target), so the
                    // bytes after the link tag go on as they came, without encoding them again.
                    let body = Bytes::from(body);
                    let encoded = body.slice(1..);
                    if let Some(epoch) = holder_only(&message) {
                        // Checked again here, as it leaves, and never delayed by the simulated network: a lease change
                        // after the display sent it must not let it reach this viewer.
                        if holds(session, viewer, epoch) {
                            send(&mut ws_tx, WsMessage::Binary(encoded)).await?;
                            if let Message::FileOffer { lease_epoch, file, size, name } = message {
                                let offered = Ticket { viewer, login, lease_epoch, file, size, name };
                                let ticket = issue(session, offered, &mut tickets);
                                let link = Message::FileLink { lease_epoch, file, ticket }.encode();
                                send(&mut ws_tx, WsMessage::Binary(link.into())).await?;
                            }
                        }
                        continue;
                    }
                    let network = *conditions.get_or_insert_with(|| session.state.lock().unwrap().network);
                    let lane = lane(&message);
                    if network == Network::default() && outbox.is_empty() {
                        // No simulated conditions: straight on, without waiting for a timer tick.
                        if !forward(&mut ws_tx, &mut path, lane, body).await? {
                            fall_back(&mut path, &mut ws_tx, &mut display_tx).await?;
                        }
                    } else if let Some(at) = simulate(&network, is_media(&message), encoded.len(), &mut next_free) {
                        outbox_bytes += body.len();
                        outbox.push_back((at, lane, body));
                    } else if let Some(revision) = message.media_revision() {
                        // Lost: the display waits for no acknowledgement of it.
                        let lost = Link::MediaDiscarded { revision };
                        send_display(&mut display_tx, &lost).await?;
                    }
                }
            }
            _ = sleep_until(due), if due.is_some() => {
                let (_, lane, body) = outbox.pop_front().expect("due implies a message");
                outbox_bytes -= body.len();
                if !forward(&mut ws_tx, &mut path, lane, body).await? {
                    fall_back(&mut path, &mut ws_tx, &mut display_tx).await?;
                }
            }
            incoming = ws_rx.next() => {
                let Some(incoming) = incoming else { return Ok(()) };
                match incoming.map_err(|e| e.to_string())? {
                    WsMessage::Binary(bytes) => {
                        let message = Message::decode(&bytes, Direction::ClientToServer).map_err(|e| format!("client: {e}"))?;
                        if message == Message::UdpRequest {
                            if !hello {
                                return Err("the first message must be hello".into());
                            }
                            // At most one offer every OFFER_EVERY, and none while the media are on a session.
                            let due = last_offer.is_none_or(|at| at.elapsed() >= OFFER_EVERY);
                            if let Some(udp) = udp.filter(|_| path.is_none() && due) {
                                (generation, last_offer) = (generation.wrapping_add(1), Some(Instant::now()));
                                offered = Some(generation);
                                let offer = udp.offer(&session.id, viewer, generation).encode();
                                send(&mut ws_tx, WsMessage::Binary(offer.into())).await?;
                            }
                            continue;
                        }
                        if let Some(link) = admit(session, viewer, message, &mut held, &mut hello)? {
                            send_display(&mut display_tx, &link).await?;
                        }
                    }
                    WsMessage::Pong(_) => last_pong = Instant::now(),
                    WsMessage::Close(_) => return Ok(()),
                    WsMessage::Text(_) => return Err("text frames are not part of the protocol".into()),
                    WsMessage::Ping(_) => {}
                }
            }
            _ = wake.notified() => {
                credit(&mut downloads, &mut display_tx, false).await?;
                deliver(session, viewer, &mut delivering, &mut ws_tx).await?;
            }
            // `changed` marks the lease seen: it is told here, as it is.
            Ok(()) = lease.changed() => {
                let now = *lease.borrow_and_update();
                tell_lease(session, viewer, now, &mut downloads, &mut display_tx, &mut ws_tx).await?;
            }
            Ok(()) = settings.changed() => {
                let current = *settings.borrow_and_update();
                send_display(&mut display_tx, &Link::Settings(current)).await?;
            }
            command = from_app.recv() => match command {
                Some(ToViewer::Download(diversion, taken)) => {
                    tell_pending_lease(session, viewer, &mut lease, &mut downloads, &mut display_tx, &mut ws_tx).await?;
                    let (file, lease_epoch) = (diversion.file, diversion.lease_epoch);
                    let free = session.features.downloads
                        && !downloads.contains_key(&file)
                        && holds(session, viewer, lease_epoch);
                    if free {
                        let read = Link::Relay(Message::FileRead { lease_epoch, file, offset: 0 });
                        send_display(&mut display_tx, &read).await?;
                        downloads.insert(file, Diverted { taken: diversion, received: 0, credited: 0 });
                    }
                    let _ = taken.send(free);
                }
                // The session of the latest offer moves the media onto it: the client takes what comes over it only
                // after this, behind every media message the WebSocket carried. An older offer's is closed.
                Some(ToViewer::Udp { generation, connection }) => {
                    if path.is_none() && offered == Some(generation) {
                        offered = None;
                        path = Some(Path::open(generation, connection));
                        tracing::info!(session = %session.id, viewer, generation, "media on the direct path");
                        send(&mut ws_tx, WsMessage::Binary(Message::MediaMoved { generation }.encode().into())).await?;
                    } else {
                        // Closed, not only dropped: HTTP/3's own task would keep it open.
                        connection.close(0u32.into(), b"");
                    }
                }
                Some(ToViewer::Open { lease_epoch, url, opened }) => {
                    tell_pending_lease(session, viewer, &mut lease, &mut downloads, &mut display_tx, &mut ws_tx).await?;
                    // Only while this viewer still holds the lease it was asked under; a few at a time.
                    // Never answered (the display lost it): given up after a while, so it holds no room.
                    opening.retain(|_, (at, _)| at.elapsed() < PAGE_OPEN_WAIT);
                    if holds(session, viewer, lease_epoch) && opening.len() < MAX_OPENING {
                        next_open = next_open.wrapping_add(1);
                        let open = Link::OpenPage { request: next_open, lease_epoch, url };
                        send_display(&mut display_tx, &open).await?;
                        opening.insert(next_open, (Instant::now(), opened));
                    } else {
                        let _ = opened.send(false);
                    }
                }
                Some(ToViewer::Close(why)) => {
                    let frame = (why == crate::app::MOVED)
                        .then(|| CloseFrame { code: crate::app::MOVED_CODE, reason: "moved".into() });
                    let _ = send(&mut ws_tx, WsMessage::Close(frame)).await;
                    return Err(why.into());
                }
                None => return Ok(()),
            },
            // The client's acknowledgements and keyframe requests over the session; its end loses the path.
            upstream = from_client(&mut path) => match upstream {
                Some(message) => {
                    if let (Message::MediaAck { revision }, Some(path)) = (&message, path.as_mut()) {
                        path.acked(*revision);
                    }
                    if let Some(link) = admit(session, viewer, message, &mut held, &mut hello)? {
                        send_display(&mut display_tx, &link).await?;
                    }
                }
                None => fall_back(&mut path, &mut ws_tx, &mut display_tx).await?,
            },
            _ = watch.tick(), if path.is_some() => {
                if path.as_ref().is_some_and(Path::stalled) {
                    fall_back(&mut path, &mut ws_tx, &mut display_tx).await?;
                }
            }
            _ = ping.tick() => {
                lost_lease(session, viewer, &mut downloads);
                deliver(session, viewer, &mut delivering, &mut ws_tx).await?;
                // A slow response still shows progress, so the display does not take it for abandoned.
                credit(&mut downloads, &mut display_tx, true).await?;
                if last_pong.elapsed() > PONG_DEADLINE {
                    return Err("the client stopped answering pings".into());
                }
                send(&mut ws_tx, WsMessage::Ping(Vec::new().into())).await?;
            }
        }
    }
}

/// Mints a single-use ticket for a download offered to this viewer, dropping its oldest past [`MAX_TICKETS`].
fn issue(session: &Session, offered: Ticket, tickets: &mut VecDeque<[u8; FILE_TICKET]>) -> [u8; FILE_TICKET] {
    let mut ticket = [0u8; FILE_TICKET];
    rand::rng().fill_bytes(&mut ticket);
    let mut state = session.state.lock().unwrap();
    while tickets.len() >= MAX_TICKETS {
        if let Some(old) = tickets.pop_front() {
            state.tickets.remove(&old);
        }
    }
    state.tickets.insert(ticket, offered);
    tickets.push_back(ticket);
    ticket
}

/// Grants credit to downloads whose responses took [`CREDIT_EVERY`] more (or anything more, when `any`), and cancels
/// those whose responses were dropped.
async fn credit(
    downloads: &mut HashMap<u32, Diverted>,
    display_tx: &mut OwnedWriteHalf,
    any: bool,
) -> Result<(), String> {
    let dropped: Vec<u32> = downloads
        .iter()
        .filter(|(_, d)| d.taken.progress.dropped.load(Ordering::Acquire))
        .map(|(file, _)| *file)
        .collect();
    for file in dropped {
        fail(downloads, file, display_tx).await?;
    }
    for (file, d) in downloads.iter_mut() {
        let consumed = d.taken.progress.consumed.load(Ordering::Acquire);
        if consumed >= d.credited + CREDIT_EVERY || (any && consumed > d.credited) {
            let read =
                Link::Relay(Message::FileRead { lease_epoch: d.taken.lease_epoch, file: *file, offset: consumed });
            send_display(display_tx, &read).await?;
            d.credited = consumed;
        }
    }
    Ok(())
}

/// Fails a download: its response errors (so the person's browser never shows it complete) and the display stops it.
async fn fail(
    downloads: &mut HashMap<u32, Diverted>,
    file: u32,
    display_tx: &mut OwnedWriteHalf,
) -> Result<(), String> {
    if let Some(d) = downloads.remove(&file) {
        let _ = d.taken.pieces.try_send(Piece::Fail);
        let cancel = Link::Relay(Message::FileCancel { lease_epoch: d.taken.lease_epoch, file });
        send_display(display_tx, &cancel).await?;
    }
    Ok(())
}

/// Writes one link message to the display.
async fn send_display(display_tx: &mut OwnedWriteHalf, link: &Link) -> Result<(), String> {
    display_tx.write_all(&link.encode()).await.map_err(|e| e.to_string())
}

/// Pages awaiting the display's answer at once, per viewer.
const MAX_OPENING: usize = 4;

/// A download whose confirmed end its response has not taken yet, with the `FileEnd` to send once it has.
struct Delivering {
    lease_epoch: u32,
    end: Bytes,
    progress: Arc<DownloadProgress>,
}

/// Tells the holder of each download whose response took its end that it ended (or, if the response was dropped
/// first, that it is gone).
async fn deliver(
    session: &Session,
    viewer: u64,
    delivering: &mut HashMap<u32, Delivering>,
    ws_tx: &mut WsSink,
) -> Result<(), String> {
    let settled: Vec<u32> = delivering
        .iter()
        .filter(|(_, d)| d.progress.delivered.load(Ordering::Acquire) || d.progress.dropped.load(Ordering::Acquire))
        .map(|(file, _)| *file)
        .collect();
    for file in settled {
        let Some(d) = delivering.remove(&file) else { continue };
        if !holds(session, viewer, d.lease_epoch) {
            continue;
        }
        let message = if d.progress.delivered.load(Ordering::Acquire) {
            d.end
        } else {
            Bytes::from(Message::FileGone { lease_epoch: d.lease_epoch, file }.encode())
        };
        send(ws_tx, WsMessage::Binary(message)).await?;
    }
    Ok(())
}

type WsSink = futures_util::stream::SplitSink<WebSocket, WsMessage>;

/// Sends one message to the client within [`PONG_DEADLINE`]: a client that stops reading would otherwise hold the relay's
/// loop, and with it the ping deadline and the lease, for as long as its connection lingers. Late ends the relay.
async fn send<S: futures_util::Sink<WsMessage, Error = axum::Error> + Unpin>(
    ws_tx: &mut S,
    message: WsMessage,
) -> Result<(), String> {
    match tokio::time::timeout(PONG_DEADLINE, ws_tx.send(message)).await {
        Ok(sent) => sent.map_err(|e| e.to_string()),
        Err(_) => Err("the client stopped reading".into()),
    }
}

/// Tells the display and the client a lease this viewer was given unless they were told it already: before a command
/// fenced by a lease epoch (a page to open, a download to read), as the queue it came through may be read before the
/// lease that came after its own, and the display refuses an epoch it was never granted.
async fn tell_pending_lease<S: futures_util::Sink<WsMessage, Error = axum::Error> + Unpin>(
    session: &Session,
    viewer: u64,
    lease: &mut watch::Receiver<(u32, LeaseHolder)>,
    downloads: &mut HashMap<u32, Diverted>,
    display_tx: &mut OwnedWriteHalf,
    ws_tx: &mut S,
) -> Result<(), String> {
    if !lease.has_changed().unwrap_or(false) {
        return Ok(());
    }
    let now = *lease.borrow_and_update();
    tell_lease(session, viewer, now, downloads, display_tx, ws_tx).await
}

/// Tells the display (its grant first) and the client the lease this viewer was given.
async fn tell_lease<S: futures_util::Sink<WsMessage, Error = axum::Error> + Unpin>(
    session: &Session,
    viewer: u64,
    (lease_epoch, holder): (u32, LeaseHolder),
    downloads: &mut HashMap<u32, Diverted>,
    display_tx: &mut OwnedWriteHalf,
    ws_tx: &mut S,
) -> Result<(), String> {
    if let Some(grant) = grant(lease_epoch, holder) {
        send_display(display_tx, &grant).await?;
    }
    lost_lease(session, viewer, downloads);
    let told = Message::Lease { lease_epoch, holder };
    send(ws_tx, WsMessage::Binary(told.encode().into())).await
}

/// What a lease this viewer is told tells the display over its link: a positive grant only to its holder, a revocation
/// over every viewer's (the display ignores one no newer than its lease), nothing while another viewer holds it.
fn grant(lease_epoch: u32, holder: LeaseHolder) -> Option<Link> {
    match holder {
        LeaseHolder::You => Some(Link::Grant { lease_epoch, holder: true }),
        LeaseHolder::Agent => Some(Link::Grant { lease_epoch, holder: false }),
        LeaseHolder::Other => None,
    }
}

/// Fails the downloads of a lease this viewer no longer holds (the display ends them too).
fn lost_lease(session: &Session, viewer: u64, downloads: &mut HashMap<u32, Diverted>) {
    downloads.retain(|_, d| {
        let held = holds(session, viewer, d.taken.lease_epoch);
        if !held {
            let _ = d.taken.pieces.try_send(Piece::Fail);
        }
        held
    });
}

/// Where a display message goes: media and sound take the direct path while it is up, everything else the WebSocket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane {
    /// A configuration, or media the client acknowledges by its revision.
    Media(Option<u64>),
    Sound,
    Control,
}

fn lane(message: &Message) -> Lane {
    match message {
        Message::Config { .. } => Lane::Media(None),
        Message::Audio { .. } => Lane::Sound,
        _ => message.media_revision().map_or(Lane::Control, |revision| Lane::Media(Some(revision))),
    }
}

/// Sends one display message (`body`, with the link's relay tag) on its path; false when the direct path could not
/// take it, which loses the path.
async fn forward(ws_tx: &mut WsSink, path: &mut Option<Path>, lane: Lane, body: Bytes) -> Result<bool, String> {
    if let Some(path) = path {
        match lane {
            Lane::Media(revision) => return Ok(path.send_media(revision, body)),
            Lane::Sound => return Ok(path.send_sound(body)),
            Lane::Control => {}
        }
    }
    send(ws_tx, WsMessage::Binary(body.slice(1..))).await?;
    Ok(true)
}

/// The direct path is lost: the client hears it on the WebSocket first, then the display opens a new epoch, whose
/// media come on the WebSocket behind that (see `Link::PathChanged`).
async fn fall_back(path: &mut Option<Path>, ws_tx: &mut WsSink, display_tx: &mut OwnedWriteHalf) -> Result<(), String> {
    let Some(lost) = path.take() else { return Ok(()) };
    let generation = lost.generation;
    drop(lost);
    tracing::info!(generation, "the direct path was lost: media back on the WebSocket");
    send(ws_tx, WsMessage::Binary(Message::MediaFallback { generation }.encode().into())).await?;
    send_display(display_tx, &Link::PathChanged).await
}

/// The next message the client sent over the direct path, `None` once its stream or session ended; never while there is
/// no path.
async fn from_client(path: &mut Option<Path>) -> Option<Message> {
    match path {
        Some(path) => path.upstream.recv().await,
        None => std::future::pending().await,
    }
}

async fn sleep_until(at: Option<Instant>) {
    if let Some(at) = at {
        tokio::time::sleep_until(at.into()).await;
    }
}

/// The lease epoch of a message meant for the lease holder alone: every message to the viewer that carries one.
fn holder_only(message: &Message) -> Option<u32> {
    if message.direction() == Direction::ServerToClient { message.lease_epoch() } else { None }
}

/// Whether `viewer` holds control at `epoch`.
fn holds(session: &Session, viewer: u64, epoch: u32) -> bool {
    session.state.lock().unwrap().holds(viewer, epoch)
}

/// Video, tiles and sound: what the simulated network may lose.
fn is_media(message: &Message) -> bool {
    matches!(message, Message::Video { .. } | Message::Tiles { .. } | Message::Audio { .. })
}

/// When a display message leaves under the simulated network, or `None` if it is lost. Only media can be lost.
fn simulate(network: &Network, media: bool, len: usize, next_free: &mut Instant) -> Option<Instant> {
    if media && network.loss_percent > 0.0 && rand::random::<f32>() * 100.0 < network.loss_percent {
        return None;
    }
    let now = Instant::now();
    let mut start = now.max(*next_free);
    if network.bandwidth_kbps > 0 {
        let bits = len as f64 * 8.0;
        start += Duration::from_secs_f64(bits / (f64::from(network.bandwidth_kbps) * 1000.0));
    }
    *next_free = start;
    Some(start + Duration::from_millis(u64::from(network.latency_ms)))
}

fn record_server(session: &Session, viewer: u64, message: &Message) {
    if !matches!(message, Message::ServerStats { .. } | Message::Config { .. }) {
        return;
    }
    let mut state = session.state.lock().unwrap();
    let Some(v) = state.viewers.get_mut(&viewer) else { return };
    match message {
        Message::ServerStats { frames, bytes, encode_us_p50, encode_us_p95, .. } => {
            v.stats.frames_per_s = *frames;
            v.stats.kbit_per_s = bytes * 8 / 1000;
            v.stats.encode_us_p50 = *encode_us_p50;
            v.stats.encode_us_p95 = *encode_us_p95;
        }
        Message::Config { codec, .. } => v.stats.codec = format!("{codec:?}"),
        _ => {}
    }
}

/// Whether a viewer's input belongs to a feature the session has on.
fn allowed(features: Features, message: &Message) -> bool {
    match message {
        Message::Pointer { .. } | Message::Wheel { .. } | Message::Key { .. } => features.keyboard_pointer,
        Message::Clipboard { .. } => features.clipboard,
        Message::UploadStart { .. }
        | Message::UploadData { .. }
        | Message::FileChoose { .. }
        | Message::FileDrop { .. }
        | Message::FileDrag { .. }
        | Message::FileDragEnd { .. } => features.uploads,
        // A recording of the page's sound needs the sound on; checked by `admit` as it goes on.
        Message::RecordStart { .. }
        | Message::RecordCamera { .. }
        | Message::RecordMicrophone { .. }
        | Message::RecordStop { .. }
        | Message::RecordLayout { .. } => features.recording,
        _ => true,
    }
}

/// What the display sends that a feature the session has off would carry: `Some` withholds it from the viewer, with
/// the answer the display is owed, if any (sent by the holder's relay alone): a download it offered is cancelled, and
/// the page's file chooser is answered with nothing chosen.
fn withheld(session: &Session, viewer: u64, message: &Message) -> Option<Option<Link>> {
    let features = session.features;
    let off = match message {
        Message::Copied { .. } => !features.clipboard,
        Message::Audio { .. } => !features.sound,
        Message::RecordState { .. } => !features.recording,
        Message::FileOffer { .. } | Message::FileData { .. } | Message::FileEnd { .. } | Message::FileGone { .. } => {
            !features.downloads
        }
        Message::FileRequest { .. }
        | Message::FileRequestEnd { .. }
        | Message::UploadAck { .. }
        | Message::UploadDone { .. } => !features.uploads,
        _ => false,
    };
    if !off {
        return None;
    }
    let answer = match *message {
        Message::FileOffer { lease_epoch, file, .. } if holds(session, viewer, lease_epoch) => {
            Some(Link::Relay(Message::FileCancel { lease_epoch, file }))
        }
        Message::FileRequest { lease_epoch, request, .. } if holds(session, viewer, lease_epoch) => {
            Some(Link::Relay(Message::FileChoose { lease_epoch, request, uploads: Vec::new() }))
        }
        _ => None,
    };
    Some(answer)
}

/// Decides what a client message becomes on the display link. Input is forwarded only for the lease holder at the
/// current epoch; refused input is dropped, not answered.
fn admit(
    session: &Session,
    viewer: u64,
    message: Message,
    held: &mut Modifiers,
    hello: &mut bool,
) -> Result<Option<Link>, String> {
    match &message {
        Message::Hello { .. } if *hello => return Err("a second hello".into()),
        Message::Hello { .. } => *hello = true,
        _ if !*hello => return Err("the first message must be hello".into()),
        Message::FileRead { .. } | Message::FileCancel { .. } => {
            tracing::warn!(session = %session.id, viewer, "dropped a viewer's own download read; downloads use tickets");
            return Ok(None);
        }
        Message::ClientStats { presented, decode_us_p50, hardware } => {
            let mut state = session.state.lock().unwrap();
            if let Some(v) = state.viewers.get_mut(&viewer) {
                v.stats.presented_per_s = *presented;
                v.stats.decode_us_p50 = *decode_us_p50;
                v.stats.hardware = *hardware;
            }
        }
        _ => {}
    }
    if !message.is_input() {
        return Ok(Some(Link::Relay(message)));
    }
    if !allowed(session.features, &message) {
        tracing::warn!(session = %session.id, viewer, "dropped input for a feature this session has off");
        return Ok(None);
    }
    let mut state = session.state.lock().unwrap();
    let epoch = message.lease_epoch().expect("input messages carry a lease epoch");
    if !state.holds(viewer, epoch) {
        tracing::warn!(session = %session.id, viewer, epoch, "dropped input from a viewer without control");
        return Ok(None);
    }
    record(&mut state.redactions, held, epoch, &message);
    let message = match message {
        Message::RecordStart { lease_epoch, recording, layout, camera, microphone, page_sound } => {
            let page_sound = page_sound && session.features.sound;
            Message::RecordStart { lease_epoch, recording, layout, camera, microphone, page_sound }
        }
        other => other,
    };
    Ok(Some(Link::Relay(message)))
}

/// Records what the holder types for redaction. A web client sends a printable character as text and a letter key's
/// physical code only for a shortcut (Ctrl, Alt or Meta held; a Mac's Command arrives as Ctrl), and every repeat of a
/// held key as a press of its own (the seat repeats nothing). `epoch` is the lease epoch the holder sent `message`
/// under.
fn record(redactions: &mut Redactions, held: &mut Modifiers, epoch: u32, message: &Message) {
    held.at(epoch);
    if let Message::Key { kind: kind @ (KeyKind::Down | KeyKind::Up), code, .. } = message
        && let Some(bit) = Modifiers::bit(*code)
    {
        if *kind == KeyKind::Down {
            held.0 |= bit;
        } else {
            held.0 &= !bit;
        }
        return;
    }
    match message {
        Message::Key { kind: KeyKind::Down, code, .. } => key_down(redactions, held, *code),
        Message::Key { kind: KeyKind::Text, text, .. } => redactions.typed(text),
        Message::Pointer { kind: display_protocol::PointerKind::Down, .. } => redactions.boundary(),
        // Pasted at the cursor, into the run typed around it.
        Message::Clipboard { text, .. } => redactions.pasted(text),
        _ => {}
    }
}

/// What one key pressed does to the typed run: an edit it follows (the cursor's moves, a character deleted on either
/// side), a character typed, a field boundary (Enter and Tab; Escape, which may clear or revert the field; and
/// Shift+Insert, which pastes the remote clipboard, a text the relay never sees), or an edit it cannot follow: a
/// selection (Shift with any move, the keypad's included, or with Delete) and any shortcut, which may select, paste,
/// undo or delete a word (see [`Redactions::lose`]).
fn key_down(redactions: &mut Redactions, held: &Modifiers, code: u16) {
    if held.chord() {
        return redactions.lose();
    }
    let edit = match code {
        KEY_BACKSPACE => Edit::Backspace,
        KEY_INSERT if held.shift() => return redactions.boundary(),
        KEY_DELETE | KEY_LEFT | KEY_RIGHT | KEY_HOME | KEY_END | KEY_UP | KEY_DOWN | KEY_PAGEUP | KEY_PAGEDOWN
            if held.shift() =>
        {
            return redactions.lose();
        }
        code if held.shift() && KEYPAD.contains(&code) => return redactions.lose(),
        KEY_DELETE => Edit::Delete,
        KEY_LEFT => Edit::Left,
        KEY_RIGHT => Edit::Right,
        KEY_HOME => Edit::Home,
        KEY_END => Edit::End,
        KEY_ENTER | KEY_KPENTER | KEY_TAB | KEY_ESC => return redactions.boundary(),
        KEY_UP | KEY_DOWN | KEY_PAGEUP | KEY_PAGEDOWN => Edit::Line,
        code if KEYPAD.contains(&code) => Edit::Line,
        code => {
            if let Some(c) = evdev_char(code, held.shift()) {
                redactions.typed(&c.to_string());
            }
            return;
        }
    };
    redactions.edit(edit);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client that stopped reading: nothing it is sent ever goes out.
    struct Stuck;

    impl futures_util::Sink<WsMessage> for Stuck {
        type Error = axum::Error;
        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), axum::Error>> {
            std::task::Poll::Pending
        }
        fn start_send(self: std::pin::Pin<&mut Self>, _: WsMessage) -> Result<(), axum::Error> {
            Ok(())
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), axum::Error>> {
            std::task::Poll::Pending
        }
        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), axum::Error>> {
            std::task::Poll::Pending
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_send_to_a_client_that_stopped_reading_ends_at_the_pong_deadline() {
        let start = tokio::time::Instant::now();
        assert!(send(&mut Stuck, WsMessage::Ping(Vec::new().into())).await.is_err());
        assert_eq!(start.elapsed(), PONG_DEADLINE);
    }

    #[tokio::test]
    async fn a_lease_not_yet_told_reaches_the_display_once_before_what_follows_it() {
        let session = Session::test("abc", crate::app::Owner::Admin, Features::default());
        let (lease_tx, mut lease) = watch::channel((0, LeaseHolder::Agent));
        let (display, mut theirs) = UnixStream::pair().unwrap();
        let (_, mut display_tx) = display.into_split();
        let mut ws_tx = Vec::new().sink_map_err(|never: std::convert::Infallible| -> axum::Error { match never {} });
        let mut downloads = HashMap::new();
        lease_tx.send_replace((3, LeaseHolder::You));
        for _ in 0..2 {
            tell_pending_lease(&session, 1, &mut lease, &mut downloads, &mut display_tx, &mut ws_tx).await.unwrap();
        }
        // What the relay writes next (a page to open, at epoch 3) follows the grant on the link.
        display_tx.write_all(b"next").await.unwrap();
        drop(display_tx);
        let mut link = Vec::new();
        theirs.read_to_end(&mut link).await.unwrap();
        let grant = Link::Grant { lease_epoch: 3, holder: true }.encode();
        assert_eq!(link, [grant.as_slice(), b"next"].concat());
        let lease = Message::Lease { lease_epoch: 3, holder: LeaseHolder::You }.encode();
        assert_eq!(ws_tx.into_inner(), [WsMessage::Binary(lease.into())]);
    }

    #[test]
    fn only_the_holder_s_link_carries_a_positive_grant() {
        assert_eq!(grant(3, LeaseHolder::You), Some(Link::Grant { lease_epoch: 3, holder: true }));
        assert_eq!(grant(5, LeaseHolder::Agent), Some(Link::Grant { lease_epoch: 5, holder: false }));
        assert_eq!(grant(3, LeaseHolder::Other), None);
    }

    #[test]
    fn media_and_sound_take_the_direct_path_and_everything_else_the_websocket() {
        let tiles = Message::Tiles { epoch: 1, revision: 9, rects: vec![], raw_len: 0, data: vec![] };
        assert_eq!(lane(&tiles), Lane::Media(Some(9)));
        let config = Message::Config {
            version: display_protocol::VERSION,
            epoch: 1,
            width: 64,
            height: 64,
            codec: display_protocol::Codec::Av1,
            max_fps: 60,
        };
        assert_eq!(lane(&config), Lane::Media(None));
        assert_eq!(lane(&Message::Audio { timestamp_us: 0, payload: vec![1] }), Lane::Sound);
        for control in [
            Message::TimePong { client_time_us: 1, server_time_us: 2 },
            Message::Lease { lease_epoch: 1, holder: display_protocol::LeaseHolder::You },
            Message::Copied { lease_epoch: 1, text: "a".into() },
        ] {
            assert_eq!(lane(&control), Lane::Control, "{control:?}");
        }
    }

    #[test]
    fn the_network_simulator_delays_shapes_and_drops_only_media() {
        let mut next_free = Instant::now();
        let start = Instant::now();
        let slow = Network { latency_ms: 100, loss_percent: 0.0, bandwidth_kbps: 8 };
        let pong = Message::TimePong { client_time_us: 1, server_time_us: 2 }.encode().len();
        let at = simulate(&slow, false, pong, &mut next_free).unwrap();
        // 17 bytes at 8 kbit/s take 17 ms, plus 100 ms of latency.
        let wait = at.duration_since(start);
        assert!(wait >= Duration::from_millis(117) && wait < Duration::from_millis(140), "{wait:?}");
        let lossy = Network { latency_ms: 0, loss_percent: 100.0, bandwidth_kbps: 0 };
        let tiles = Message::Tiles { epoch: 1, revision: 9, rects: vec![], raw_len: 0, data: vec![] };
        assert!(is_media(&tiles));
        // A lost video or tiles message is reported to the display by its revision; lost sound needs no report.
        assert_eq!(tiles.media_revision(), Some(9));
        assert_eq!(Message::Audio { timestamp_us: 0, payload: vec![1] }.media_revision(), None);
        assert!(simulate(&lossy, true, 64, &mut next_free).is_none());
        assert!(simulate(&lossy, false, pong, &mut next_free).is_some());
    }

    fn session(features: Features) -> Session {
        Session::test("abc", crate::app::Owner::Admin, features)
    }

    #[test]
    fn every_message_to_the_viewer_that_carries_a_lease_epoch_is_for_the_holder_alone() {
        use display_protocol::RecordPhase;
        let state = Message::RecordState {
            lease_epoch: 5,
            recording: 1,
            phase: RecordPhase::Recording,
            file: 2,
            seconds: 3,
            bytes: 4,
        };
        assert_eq!(holder_only(&state), Some(5));
        assert_eq!(holder_only(&Message::Copied { lease_epoch: 7, text: "x".into() }), Some(7));
        assert_eq!(holder_only(&Message::Lease { lease_epoch: 9, holder: display_protocol::LeaseHolder::Agent }), None);
        // The holder's own input carries its epoch too, but goes the other way.
        let key = Message::Key { lease_epoch: 3, kind: KeyKind::Down, code: 30, text: String::new() };
        assert_eq!(holder_only(&key), None);
    }

    #[test]
    fn a_shortcut_ends_the_typed_run_instead_of_adding_its_letter() {
        let key = |kind, code| Message::Key { lease_epoch: 1, kind, code, text: String::new() };
        let mut redactions = Redactions::default();
        let mut held = Modifiers::default();
        let typed = Message::Key { lease_epoch: 1, kind: KeyKind::Text, code: 0, text: "secret-value".into() };
        for message in [
            typed,
            key(KeyKind::Down, KEY_LEFTCTRL),
            key(KeyKind::Down, 30),
            key(KeyKind::Up, 30),
            key(KeyKind::Up, KEY_LEFTCTRL),
            key(KeyKind::Down, KEY_TAB),
        ] {
            record(&mut redactions, &mut held, 1, &message);
        }
        let page = redactions.applied(serde_json::json!("field secret-value"));
        assert_eq!(page, "field [redacted]");
        // With Ctrl released, the letter key types again (Shift held picks its capital).
        record(&mut redactions, &mut held, 1, &key(KeyKind::Down, KEY_RIGHTSHIFT));
        for code in [30, 30, 30, 30] {
            record(&mut redactions, &mut held, 1, &key(KeyKind::Down, code));
        }
        record(&mut redactions, &mut held, 1, &key(KeyKind::Up, KEY_RIGHTSHIFT));
        record(&mut redactions, &mut held, 1, &key(KeyKind::Down, KEY_ENTER));
        let page = redactions.applied(serde_json::json!("AAAA"));
        assert_eq!(page, "[redacted]");
    }

    /// What the holder's browser sends for `steps` (text, or a key pressed and released), then Enter; and whether
    /// `page`, the value the field really holds then, is hidden whole from an agent's read.
    fn hides(steps: &[Step], page: &str) -> bool {
        read(steps, page) == "[redacted]"
    }

    /// What an agent's read of `page` gives after the holder's browser sent `steps`, then Enter.
    fn read(steps: &[Step], page: &str) -> serde_json::Value {
        let mut redactions = Redactions::default();
        let mut held = Modifiers::default();
        let key = |kind, code| Message::Key { lease_epoch: 1, kind, code, text: String::new() };
        let mut messages = Vec::new();
        for step in steps {
            match step {
                Step::Text(text) => {
                    messages.push(Message::Key { lease_epoch: 1, kind: KeyKind::Text, code: 0, text: (*text).into() })
                }
                // A held key: its press, then each browser repeat as its release and a new press, then its release.
                Step::Hold(code, presses) => {
                    messages.push(key(KeyKind::Down, *code));
                    for _ in 1..*presses {
                        messages.extend([key(KeyKind::Up, *code), key(KeyKind::Down, *code)]);
                    }
                    messages.push(key(KeyKind::Up, *code));
                }
                Step::Chord(modifier, code) => messages.extend([
                    key(KeyKind::Down, *modifier),
                    key(KeyKind::Down, *code),
                    key(KeyKind::Up, *code),
                    key(KeyKind::Up, *modifier),
                ]),
                // The client sends Ctrl down, swallows the V, and sends the clipboard.
                Step::Paste(text) => messages.extend([
                    key(KeyKind::Down, KEY_LEFTCTRL),
                    Message::Clipboard { lease_epoch: 1, text: (*text).into() },
                    key(KeyKind::Up, KEY_LEFTCTRL),
                ]),
            }
        }
        messages.push(key(KeyKind::Down, KEY_ENTER));
        for message in &messages {
            record(&mut redactions, &mut held, 1, message);
        }
        redactions.applied(serde_json::json!(page))
    }

    enum Step {
        Text(&'static str),
        Hold(u16, usize),
        Chord(u16, u16),
        Paste(&'static str),
    }

    #[test]
    fn a_held_backspace_sent_as_repeats_deletes_as_many_characters() {
        use Step::*;
        let steps = [Text("correct-horse-staple"), Hold(KEY_BACKSPACE, 6), Text("battery-9")];
        assert!(hides(&steps, "correct-horse-battery-9"));
    }

    #[test]
    fn typing_after_the_cursor_moved_inserts_where_the_field_does() {
        use Step::*;
        assert!(hides(&[Text("hunter2wifi"), Hold(KEY_LEFT, 4), Text("-")], "hunter2-wifi"));
        assert!(hides(&[Text("hunte2"), Hold(KEY_LEFT, 2), Hold(KEY_RIGHT, 1), Text("r")], "hunter2"));
    }

    #[test]
    fn what_is_typed_after_a_move_out_of_the_run_is_a_run_of_its_own() {
        use Step::*;
        // The field held "@corp.com"; the person clicked at its end, typed "x", pressed Home and typed a user name.
        let seen = read(&[Text("x"), Hold(KEY_HOME, 1), Text("s3cretuser")], "s3cretuser@corp.comx");
        assert_eq!(seen, "[redacted]@corp.comx");
        // Left past the run's start, into what the field held before it.
        assert_eq!(
            read(&[Text("abc"), Hold(KEY_LEFT, 5), Text("SECRET-1")], "prefilSECRET-1ledabc"),
            "prefil[redacted]ledabc"
        );
        assert!(hides(&[Text("wifi"), Hold(KEY_LEFT, 9), Text("hunter2-")], "hunter2-wifi"));
        assert!(hides(&[Text("hunter2"), Hold(KEY_HOME, 1), Hold(KEY_END, 1), Text("-wifi")], "hunter2-wifi"));
        // Each piece of the run is kept as it ends, should a move bring the cursor back into it; what is left of a
        // piece shorter than four, here before Home and the moves back in, is the stated residual.
        assert_eq!(
            read(&[Text("hunter2"), Hold(KEY_HOME, 1), Hold(KEY_RIGHT, 3), Text("X")], "hunXter2"),
            "hunX[redacted]"
        );
        assert_eq!(read(&[Text("unter2"), Hold(KEY_HOME, 1), Text("h")], "hunter2"), "h[redacted]");
    }

    #[test]
    fn delete_removes_the_character_after_the_cursor() {
        use Step::*;
        assert!(hides(&[Text("hunter2xwifi"), Hold(KEY_LEFT, 5), Hold(KEY_DELETE, 1), Text("-")], "hunter2-wifi"));
        assert!(hides(&[Text("hunter2-wifi!"), Hold(KEY_DELETE, 3)], "hunter2-wifi!"), "nothing after the run");
        assert!(hides(&[Text("pass"), Hold(KEY_BACKSPACE, 9), Text("hunter2")], "hunter2"), "nothing before it");
    }

    #[test]
    fn whatever_a_word_deletion_left_of_the_run_stays_hidden() {
        use Step::*;
        let steps = [Text("correct horse battery staple"), Chord(KEY_LEFTCTRL, KEY_BACKSPACE), Text("pencil")];
        assert!(hides(&steps, "correct horse battery pencil"));
        let steps =
            [Text("correct staple horse"), Hold(KEY_LEFT, 12), Chord(KEY_RIGHTCTRL, KEY_DELETE), Text("battery")];
        assert!(hides(&steps, "correct battery horse"));
        // The run as it stood is kept too.
        assert!(hides(&steps, "correct staple horse"));
        // Chrome's word rules keep "can't" one word: whatever the page deleted, the rest stays hidden, since every
        // piece of the run is kept once a word deletion makes the cursor unknown.
        let steps = [Text("correct horse can't"), Chord(KEY_LEFTCTRL, KEY_BACKSPACE), Text("pencil")];
        assert!(hides(&steps, "correct horse pencil") && hides(&steps, "correct horse can'pencil"));
        // Then Backspace: Chrome holds "correct horse".
        let then = [Text("correct horse can't"), Chord(KEY_LEFTCTRL, KEY_BACKSPACE), Hold(KEY_BACKSPACE, 1)];
        assert!(hides(&then, "correct horse") && hides(&then, "correct horse can"));
        let then = [Text("can't stop here"), Hold(KEY_HOME, 1), Chord(KEY_LEFTCTRL, KEY_DELETE), Hold(KEY_DELETE, 1)];
        assert!(hides(&then, "stop here") && hides(&then, "t stop here"));
        // Moves into the text before, then typing there: what is left of a piece shorter than four is the residual.
        let steps = [Text("correct horse can't"), Chord(KEY_LEFTCTRL, KEY_BACKSPACE), Hold(KEY_LEFT, 3), Text("XYZW")];
        assert!(hides(&steps, "correct horXYZWse c"));
        // Typing after the deletion, with a word of its own, and deleting a word of it.
        let steps = [
            Text("open sesame can't"),
            Chord(KEY_LEFTCTRL, KEY_BACKSPACE),
            Text("one two"),
            Chord(KEY_LEFTCTRL, KEY_BACKSPACE),
            Text("three"),
        ];
        assert!(hides(&steps, "open sesame one three") && hides(&steps, "open sesame can'one three"));
        // Deletions over text typed between them, and Home then Delete.
        let steps = [
            Text("alpha bravo charlie delta"),
            Chord(KEY_LEFTCTRL, KEY_BACKSPACE),
            Text("xyzw"),
            Chord(KEY_LEFTCTRL, KEY_BACKSPACE),
            Chord(KEY_LEFTCTRL, KEY_BACKSPACE),
        ];
        assert!(hides(&steps, "alpha bravo "));
        let steps = [
            Text("alpha bravo charlie delta"),
            Chord(KEY_LEFTCTRL, KEY_BACKSPACE),
            Hold(KEY_HOME, 1),
            Hold(KEY_DELETE, 1),
        ];
        assert!(hides(&steps, "lpha bravo charlie "));
        let steps = [
            Text("can't stop here"),
            Hold(KEY_HOME, 1),
            Chord(KEY_LEFTCTRL, KEY_DELETE),
            Hold(KEY_END, 1),
            Text("then"),
        ];
        assert!(hides(&steps, " stop herethen") && hides(&steps, "'t stop herethen"));
        // Twice: Chrome holds "correct " where the run holds "correct horse ".
        let twice =
            [Text("correct horse can't"), Chord(KEY_LEFTCTRL, KEY_BACKSPACE), Chord(KEY_LEFTCTRL, KEY_BACKSPACE)];
        assert!(hides(&twice, "correct "));
        let twice = [
            Text("can't stop here"),
            Hold(KEY_HOME, 1),
            Chord(KEY_LEFTCTRL, KEY_DELETE),
            Chord(KEY_LEFTCTRL, KEY_DELETE),
        ];
        assert!(hides(&twice, " here"), "Chrome holds \" here\"");
        let steps = [Text("can't stop here"), Hold(KEY_HOME, 1), Chord(KEY_LEFTCTRL, KEY_DELETE), Text("would")];
        assert!(hides(&steps, "would stop here") && hides(&steps, "would't stop here"));
        // Edits after the deletion: Backspace takes from what was typed since, and a move splits it into pieces that
        // stay hidden too.
        let steps = [
            Text("hunter2-wifi"),
            Chord(KEY_LEFTCTRL, KEY_BACKSPACE),
            Text("lannisterr"),
            Hold(KEY_BACKSPACE, 1),
            Hold(KEY_LEFT, 4),
            Text("wolf"),
        ];
        assert!(hides(&steps, "hunter2-lanniwolfster") && hides(&steps, "lanniwolfster"));
    }

    #[test]
    fn a_paste_goes_into_the_run_at_its_cursor() {
        use Step::*;
        const KEY_Z: u16 = 44;
        // In a plain field the person types, moves back four, pastes, and gives control back: the field holds
        // hunter2WXYZwifi, of which no piece of four characters may reach the agent.
        let value = "hunter2WXYZwifi";
        let seen = read(&[Text("hunter2wifi"), Hold(KEY_LEFT, 4), Paste("WXYZ")], &format!("value {value} end"));
        assert_eq!(seen, "value [redacted] end");
        let seen = seen.as_str().unwrap();
        let piece = crate::redact::MIN_LENGTH;
        assert!((0..=value.len() - piece).all(|at| !seen.contains(&value[at..at + piece])), "{seen}");
        assert!(hides(&[Text("sk_live_"), Paste("a8F3kQ9zR2"), Text("_prod")], "sk_live_a8F3kQ9zR2_prod"));
        // The paste is an entry of its own too, and the edits after it are followed through it.
        assert!(hides(&[Text("hunter2wifi"), Hold(KEY_LEFT, 4), Paste("WXYZ")], "WXYZ"));
        let steps = [Text("hunter2wifi"), Hold(KEY_LEFT, 4), Paste("WXYZ"), Hold(KEY_BACKSPACE, 2), Text("-")];
        assert!(hides(&steps, "hunter2WX-wifi"));
        // An undo takes the paste back out, or what was typed after a move.
        assert!(hides(&[Text("4071"), Hold(KEY_LEFT, 2), Paste("WXYZ"), Chord(KEY_LEFTCTRL, KEY_Z)], "4071"));
        assert!(hides(&[Text("4071"), Hold(KEY_LEFT, 2), Text("X"), Chord(KEY_LEFTCTRL, KEY_Z)], "4071"));
        // A single-line field drops a pasted line break, a multi-line one keeps it.
        let steps = [Text("hunter2wifi"), Hold(KEY_LEFT, 4), Paste("line1\nline2"), Hold(KEY_BACKSPACE, 1)];
        assert!(hides(&steps, "hunter2line1linewifi"));
        assert_eq!(read(&steps, "hunter2line1\nlinewifi"), "[redacted]");
    }

    #[test]
    fn after_an_edit_over_characters_outside_ascii_every_piece_of_the_run_stays_hidden() {
        use Step::*;
        // Chrome moves over the emoji sequence whole: it holds "hunter-👩‍💻wifi".
        let steps = [Text("hunter\u{1f469}\u{200d}\u{1f4bb}wifi"), Hold(KEY_LEFT, 5), Text("-")];
        assert!(!hides(&steps, "hunter-\u{1f469}\u{200d}\u{1f4bb}wifi"));
        let seen = read(&steps, "value hunter-\u{1f469}\u{200d}\u{1f4bb}wifi end");
        assert_eq!(seen, "value [redacted]-[redacted] end", "the emoji too, as a piece of what was typed");
        // An accented letter's marks: whatever Backspace removes, the rest stays hidden.
        let steps = [Text("mocha-cafe\u{301}-latte"), Hold(KEY_LEFT, 6), Hold(KEY_BACKSPACE, 1)];
        assert!(hides(&steps, "mocha-caf-latte"));
        // A move forward over a letter its marks follow.
        let steps = [Text("hunter-e\u{301}-wifi"), Hold(KEY_HOME, 1), Hold(KEY_RIGHT, 8), Text("X")];
        assert_eq!(read(&steps, "value hunter-e\u{301}X-wifi end"), "value [redacted]X[redacted] end");
    }

    #[test]
    fn what_is_typed_around_a_selection_a_shortcut_or_a_move_to_another_line_stays_hidden() {
        let mut redactions = Redactions::default();
        let mut held = Modifiers::default();
        let key = |kind, code| Message::Key { lease_epoch: 1, kind, code, text: String::new() };
        let text = |text: &str| Message::Key { lease_epoch: 1, kind: KeyKind::Text, code: 0, text: text.into() };
        for message in [
            text("first-line"),
            key(KeyKind::Down, KEY_UP),
            text("second"),
            key(KeyKind::Down, KEY_LEFTSHIFT),
            key(KeyKind::Down, KEY_LEFT),
            key(KeyKind::Up, KEY_LEFTSHIFT),
            text("third"),
            key(KeyKind::Down, 75),
            text("fourth"),
            key(KeyKind::Down, KEY_LEFTALT),
            key(KeyKind::Down, KEY_BACKSPACE),
            key(KeyKind::Up, KEY_LEFTALT),
            text("fifth"),
            key(KeyKind::Down, KEY_PAGEDOWN),
        ] {
            record(&mut redactions, &mut held, 1, &message);
        }
        let read = redactions.applied(serde_json::json!("first-line second third fourth fifth"));
        assert_eq!(read, "[redacted] [redacted] [redacted] [redacted] [redacted]");
    }

    #[test]
    fn what_is_typed_after_a_move_to_another_line_leaves_no_piece_of_the_run() {
        use Step::*;
        // In a single-line field Down goes to the end, where the moves back and the "-" land: every piece of the run is
        // hidden around it.
        for code in [KEY_DOWN, KEY_UP, KEY_PAGEDOWN, KEY_PAGEUP, 79] {
            let steps = [Text("hunter2wifi"), Hold(code, 1), Hold(KEY_LEFT, 4), Text("-")];
            assert_eq!(read(&steps, "hunter2-wifi"), "[redacted]-[redacted]", "{code}");
        }
    }

    #[test]
    fn escape_and_shift_insert_end_the_run() {
        use Step::*;
        // Escape may clear the field (a search field) or revert it (a combobox): what is typed next is a run of its own.
        let steps = [Text("hunter2"), Hold(KEY_ESC, 1), Text("secretword")];
        assert_eq!(read(&steps, "secretword hunter2"), "[redacted] [redacted]");
        // Shift+Insert pastes the remote clipboard ("XYZ" here) at the cursor, which the run never sees.
        let steps = [Text("hunter2"), Chord(KEY_LEFTSHIFT, KEY_INSERT), Text("secretword")];
        assert_eq!(read(&steps, "hunter2XYZsecretword"), "[redacted]XYZ[redacted]");
    }

    #[test]
    fn shift_with_any_move_is_a_selection() {
        use Step::*;
        // Shift+Down selects from the cursor to the end; the character typed replaces "wifi".
        for code in [KEY_END, KEY_DOWN, KEY_UP, KEY_PAGEDOWN, KEY_PAGEUP, 79] {
            let steps = [Text("hunter2wifi"), Hold(KEY_LEFT, 4), Chord(KEY_LEFTSHIFT, code), Text("!")];
            assert_eq!(read(&steps, "hunter2!"), "[redacted]!", "{code}");
        }
    }

    /// A session with viewer 1 connected, its hello admitted: what [`admit`] then needs of the relay.
    fn watched() -> (Session, Modifiers, bool) {
        let session = session(Features::default());
        session.state.lock().unwrap().viewers.insert(1, Viewer::test().0);
        let (mut held, mut hello) = (Modifiers::default(), false);
        admit(&session, 1, Message::Hello { version: display_protocol::VERSION, codecs: 0 }, &mut held, &mut hello)
            .unwrap();
        (session, held, hello)
    }

    #[test]
    fn the_holders_input_at_another_epoch_is_dropped() {
        let (session, mut held, mut hello) = watched();
        let key = |lease_epoch| Message::Key { lease_epoch, kind: KeyKind::Down, code: 30, text: String::new() };
        Session::set_holder(&mut session.state.lock().unwrap(), Some(1));
        Session::set_holder(&mut session.state.lock().unwrap(), None);
        let epoch = Session::set_holder(&mut session.state.lock().unwrap(), Some(1));
        for stale in [epoch - 4, epoch - 2, epoch + 2] {
            assert!(admit(&session, 1, key(stale), &mut held, &mut hello).unwrap().is_none(), "{stale}");
        }
        assert!(admit(&session, 1, key(epoch), &mut held, &mut hello).unwrap().is_some());
    }

    #[test]
    fn keys_held_when_control_was_lost_are_up_once_it_is_taken_back() {
        let (session, mut held, mut hello) = watched();
        let mut input = |message: Message| admit(&session, 1, message, &mut held, &mut hello).unwrap();
        let key = |lease_epoch, kind, code| Message::Key { lease_epoch, kind, code, text: String::new() };
        let mut epoch = Session::set_holder(&mut session.state.lock().unwrap(), Some(1));
        // Ctrl goes down and control is lost (given back, here) before its release is sent.
        assert!(input(key(epoch, KeyKind::Down, KEY_LEFTCTRL)).is_some());
        assert!(input(key(epoch, KeyKind::Down, KEY_LEFTSHIFT)).is_some());
        Session::set_holder(&mut session.state.lock().unwrap(), None);
        assert!(input(key(epoch, KeyKind::Up, KEY_LEFTCTRL)).is_none(), "a release at the old epoch is dropped");
        epoch = Session::set_holder(&mut session.state.lock().unwrap(), Some(1));
        let typed = Message::Key { lease_epoch: epoch, kind: KeyKind::Text, code: 0, text: "secretX".into() };
        for message in [typed, key(epoch, KeyKind::Down, KEY_BACKSPACE), key(epoch, KeyKind::Down, KEY_TAB)] {
            assert!(input(message).is_some());
        }
        // Shift is up too: the letter key types its lower case.
        for code in [30, 30, 30, 30] {
            input(key(epoch, KeyKind::Down, code));
        }
        input(key(epoch, KeyKind::Down, KEY_ENTER));
        let state = session.state.lock().unwrap();
        let page = state.redactions.applied(serde_json::json!("value secret, value secretX and aaaa"));
        // The X Backspace deleted stays hidden too, should an undo bring it back.
        assert_eq!(page, "value [redacted], value [redacted] and [redacted]");
    }

    #[test]
    fn input_for_a_feature_that_is_off_is_not_allowed() {
        let none = Features {
            keyboard_pointer: false,
            clipboard: false,
            uploads: false,
            downloads: false,
            sound: false,
            recording: false,
        };
        let key = Message::Key { lease_epoch: 1, kind: KeyKind::Down, code: 30, text: String::new() };
        let paste = Message::Clipboard { lease_epoch: 1, text: "x".into() };
        let drop = Message::FileDrop { lease_epoch: 1, x: 0, y: 0, uploads: vec![1] };
        let stop = Message::RecordStop { lease_epoch: 1, recording: 1 };
        let stats = Message::ClientStats { presented: 1, decode_us_p50: 1, hardware: false };
        for message in [&key, &paste, &drop, &stop] {
            assert!(allowed(Features::default(), message));
            assert!(!allowed(none, message));
        }
        assert!(allowed(none, &stats), "watching needs no feature");
    }

    #[test]
    fn what_a_feature_that_is_off_carries_is_withheld_and_the_display_answered_by_the_holder() {
        let off = session(Features {
            downloads: false,
            uploads: false,
            sound: false,
            clipboard: false,
            ..Features::default()
        });
        assert!(!off.features.recording, "a recording is a download");
        let offer = Message::FileOffer { lease_epoch: 1, file: 7, size: None, name: "a".into() };
        let request = Message::FileRequest { lease_epoch: 1, request: 3, multiple: false };
        let audio = Message::Audio { timestamp_us: 0, payload: vec![] };
        let copied = Message::Copied { lease_epoch: 1, text: "x".into() };
        // Not the holder: withheld, and nothing owed by this relay.
        for message in [&offer, &request, &audio, &copied] {
            assert_eq!(withheld(&off, 1, message), Some(None));
        }
        {
            let mut state = off.state.lock().unwrap();
            state.holder = Some(1);
        }
        let cancel = Link::Relay(Message::FileCancel { lease_epoch: 1, file: 7 });
        assert_eq!(withheld(&off, 1, &offer), Some(Some(cancel)));
        let nothing = Link::Relay(Message::FileChoose { lease_epoch: 1, request: 3, uploads: vec![] });
        assert_eq!(withheld(&off, 1, &request), Some(Some(nothing)));
        let on = session(Features::default());
        for message in [&offer, &request, &audio, &copied] {
            assert_eq!(withheld(&on, 1, message), None);
        }
    }
}
