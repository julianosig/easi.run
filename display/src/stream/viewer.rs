//! One viewer connection from the admin application: a reader, an encoder (see `encoder`), and a writer thread, and
//! the mailbox through which the compositor hands the encoder its work.
//!
//! The encoder takes the latest composed frame and the damage accumulated since it last encoded. While the writer's
//! queue is over its byte budget the encoder does not capture at all, so a slow viewer skips frames before they are
//! encoded (never dropping an encoded inter frame) and its dirty tiles keep accumulating until the queue drains. A
//! fast viewer has its own encoder and is unaffected.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use display_protocol::frame::{self, CodecChoice, FrameReader, Link, MAX_CHUNK};
use display_protocol::{Direction, Message};

use super::cursor::Pointer;
use super::encoder::Encoder;
use super::tiles::TileSet;
use super::{Event, Snapshot};
use crate::compose::Rect;

/// Encoded bytes a viewer may have queued before its encoder stops capturing.
pub(super) const QUEUE_BUDGET: usize = 2 * 1024 * 1024;
/// A write that cannot finish in this time means the viewer is gone.
const SEND_DEADLINE: Duration = Duration::from_secs(5);
/// The first frame from the admin application must arrive within this time.
const ATTACH_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub codec: CodecChoice,
    pub bitrate_kbps: u32,
    pub max_fps: u8,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            codec: CodecChoice::Auto,
            bitrate_kbps: frame::DEFAULT_BITRATE_KBPS,
            max_fps: frame::DEFAULT_MAX_FPS,
        }
    }
}

/// Work waiting for the encoder.
pub(super) struct Pending {
    pub(super) latest: Option<Arc<Snapshot>>,
    pub(super) dirty: TileSet,
    /// The client asked for a keyframe and a full refresh.
    pub(super) refresh: bool,
    /// Codecs the client can decode, once its hello arrived.
    pub(super) hello: Option<u8>,
    pub(super) settings: Settings,
    pub(super) settings_changed: bool,
    /// The pointer to send, latest wins; sent again after every new configuration.
    pub(super) pointer: Option<Pointer>,
    pub(super) pointer_dirty: bool,
    pub(super) closed: bool,
    /// When each recent frame was composed and what it damaged, oldest first, over at most [`COMPOSITION_HISTORY`]
    /// (or [`MAX_COMPOSITIONS`] frames): the rate the source gives each part of the screen its frames at.
    pub(super) compositions: VecDeque<(Instant, Vec<Rect>)>,
}

/// How long the compositions' history goes back.
pub(super) const COMPOSITION_HISTORY: Duration = Duration::from_secs(2);
/// The most compositions kept, whatever their age.
const MAX_COMPOSITIONS: usize = 512;

/// `Mailbox::client_presented` before a report, and once the encoder has read it.
pub(super) const NO_REPORT: u32 = u32::MAX;

pub struct Mailbox {
    pub(super) pending: Mutex<Pending>,
    pub(super) wake: Condvar,
    pub(super) queued: AtomicUsize,
    /// Bytes of file pieces queued, counted apart so they never hold back media.
    pub(super) bulk_queued: AtomicUsize,
    /// Bytes of sound queued, counted apart: sound goes out ahead of media.
    pub(super) sound_queued: AtomicUsize,
    /// The configuration epoch the encoder last announced.
    pub(super) epoch: std::sync::atomic::AtomicU32,
    /// The client has its first configuration, so it may receive sound.
    pub(super) configured: std::sync::atomic::AtomicBool,
    /// The frames the client presented in its latest one-second report, or [`NO_REPORT`].
    pub(super) client_presented: std::sync::atomic::AtomicU32,
    pub width: i32,
    pub height: i32,
}

impl Mailbox {
    pub fn new(width: i32, height: i32) -> Mailbox {
        Mailbox {
            pending: Mutex::new(Pending {
                latest: None,
                dirty: TileSet::new(width, height),
                refresh: true,
                hello: None,
                settings: Settings::default(),
                settings_changed: false,
                pointer: None,
                pointer_dirty: false,
                closed: false,
                compositions: VecDeque::new(),
            }),
            wake: Condvar::new(),
            queued: AtomicUsize::new(0),
            bulk_queued: AtomicUsize::new(0),
            sound_queued: AtomicUsize::new(0),
            epoch: std::sync::atomic::AtomicU32::new(0),
            configured: std::sync::atomic::AtomicBool::new(false),
            client_presented: std::sync::atomic::AtomicU32::new(NO_REPORT),
            width,
            height,
        }
    }

    /// Offers a newly composed frame; its damage accumulates until the encoder takes it.
    pub fn offer(&self, snapshot: Arc<Snapshot>) {
        let mut pending = self.pending.lock().unwrap();
        for rect in &snapshot.damage {
            pending.dirty.mark(rect);
        }
        let now = Instant::now();
        pending.compositions.push_back((now, snapshot.damage.clone()));
        while pending.compositions.len() > MAX_COMPOSITIONS
            || pending.compositions.front().is_some_and(|(t, _)| now.duration_since(*t) > COMPOSITION_HISTORY)
        {
            pending.compositions.pop_front();
        }
        pending.latest = Some(snapshot);
        drop(pending);
        self.wake.notify_one();
    }

    /// Offers the pointer as it is now; it goes out ahead of any frame.
    pub fn offer_pointer(&self, pointer: Pointer) {
        let mut pending = self.pending.lock().unwrap();
        pending.pointer = Some(pointer);
        pending.pointer_dirty = true;
        drop(pending);
        self.wake.notify_one();
    }

    pub fn close(&self) {
        self.pending.lock().unwrap().closed = true;
        self.wake.notify_all();
    }

    pub fn is_closed(&self) -> bool {
        self.pending.lock().unwrap().closed
    }
}

/// How often an idle writer looks whether its viewer's mailbox closed.
const CLOSED_CHECK: Duration = Duration::from_millis(500);

/// A viewer with more than this queued gets no out-of-band message until it catches up.
const SIDE_BUDGET: usize = 256 * 1024;
/// Bytes of sound that may wait for one viewer (about two seconds at the encoder's bit rate): sound goes out ahead of
/// media, so only a viewer that stopped reading gets there.
const SOUND_BUDGET: usize = 16 * 1024;
/// Bytes of file pieces that may wait for one viewer.
const BULK_BUDGET: usize = 1024 * 1024;
/// Bytes queued for one viewer past which a control message disconnects it instead (media alone stays far below).
const CONTROL_LIMIT: usize = 4 * QUEUE_BUDGET;
/// Bytes queued for one viewer past which a notice it did not ask for is dropped: half the control limit, so notices
/// alone never bring a viewer's queue near the limit at which its own replies disconnect it.
const NOTICE_BUDGET: usize = CONTROL_LIMIT / 2;

/// Every viewer's mailbox and writer queue, for messages that come from outside its encoder: the browser's sound to
/// all of them, and what only the lease holder gets.
#[derive(Default)]
pub struct Viewers {
    viewers: Mutex<Vec<Registered>>,
}

/// A viewer's connection, mailbox, and writer queue.
type Registered = (u64, Arc<Mailbox>, Outbox);

impl Viewers {
    fn add(&self, conn: u64, mailbox: Arc<Mailbox>, out: Outbox) {
        self.viewers.lock().unwrap().push((conn, mailbox, out));
    }

    /// Forgets the viewers that closed, so their writers end; called when one closes, whatever else flows.
    pub fn prune(&self) {
        self.viewers.lock().unwrap().retain(|(_, mailbox, _)| !mailbox.is_closed());
    }

    /// Offers every open viewer a composed frame, made only when one is open.
    pub fn offer(&self, snapshot: impl FnOnce() -> Arc<Snapshot>) {
        let viewers = self.viewers.lock().unwrap();
        let mut open = viewers.iter().map(|(_, mailbox, _)| mailbox).filter(|mailbox| !mailbox.is_closed()).peekable();
        if open.peek().is_none() {
            return;
        }
        let shared = snapshot();
        for mailbox in open {
            mailbox.offer(shared.clone());
        }
    }

    /// Offers every open viewer the pointer as it is now.
    pub fn offer_pointer(&self, pointer: &Pointer) {
        for (_, mailbox, _) in self.viewers.lock().unwrap().iter().filter(|(_, mailbox, _)| !mailbox.is_closed()) {
            mailbox.offer_pointer(pointer.clone());
        }
    }

    /// Whether any viewer is connected.
    pub fn any(&self) -> bool {
        self.prune();
        !self.viewers.lock().unwrap().is_empty()
    }

    /// Sends a packet of the browser's sound to every viewer that has its configuration, ahead of its media, unless
    /// [`SOUND_BUDGET`] bytes of sound already wait for it.
    pub fn broadcast_sound(&self, message: &Message) {
        let frame = Link::relay(message);
        for (_, mailbox, out) in self.viewers.lock().unwrap().iter() {
            if !mailbox.configured.load(Ordering::Relaxed)
                || mailbox.sound_queued.load(Ordering::Relaxed) + frame.len() > SOUND_BUDGET
            {
                continue;
            }
            mailbox.sound_queued.fetch_add(frame.len(), Ordering::Relaxed);
            if out.send(Out::Sound(frame.clone())).is_err() {
                mailbox.sound_queued.fetch_sub(frame.len(), Ordering::Relaxed);
            }
        }
    }

    /// Runs `send` on the viewer `conn` while it is open; false when there is none or `send` says it did not go.
    fn send_with(&self, conn: u64, send: impl FnOnce(&Mailbox, &Outbox) -> bool) -> bool {
        let viewers = self.viewers.lock().unwrap();
        viewers
            .iter()
            .find(|(c, mailbox, _)| *c == conn && !mailbox.is_closed())
            .is_some_and(|(_, m, out)| send(m, out))
    }

    /// As [`Viewers::send_with`], for a viewer that has its first configuration.
    fn send_configured(&self, conn: u64, send: impl FnOnce(&Mailbox, &Outbox) -> bool) -> bool {
        self.send_with(conn, |mailbox, out| mailbox.configured.load(Ordering::Relaxed) && send(mailbox, out))
    }

    /// Queues a piece of a file for the viewer `conn`, behind its media, unless [`BULK_BUDGET`] bytes of pieces already
    /// wait (a client that keeps one request in flight per transfer never gets there); false when it did not go.
    pub fn send_bulk_to(&self, conn: u64, message: &Message) -> bool {
        self.send_configured(conn, |mailbox, out| {
            mailbox.bulk_queued.load(Ordering::Relaxed) <= BULK_BUDGET && send_bulk(out, mailbox, message)
        })
    }

    /// A link control (not a viewer message) for the admin on connection `conn`.
    pub fn send_link_to(&self, conn: u64, link: &Link) -> bool {
        self.send_with(conn, |mailbox, out| {
            let frame = link.encode();
            mailbox.queued.fetch_add(frame.len(), Ordering::Relaxed);
            out.send(Out::Now(frame)).is_ok()
        })
    }

    /// Sends a small control message the viewer `conn` asked for (an acknowledgement, a result) to it alone, whatever
    /// its media queue holds, since the transfer it belongs to waits on it. A viewer whose queue passes
    /// [`CONTROL_LIMIT`] (it stopped reading while it kept asking) is disconnected instead. What the viewer did not ask
    /// for goes by [`Viewers::send_notice_to`].
    pub fn send_control_to(&self, conn: u64, message: &Message) -> bool {
        self.send_configured(conn, |mailbox, out| {
            if mailbox.queued.load(Ordering::Relaxed) > CONTROL_LIMIT {
                tracing::warn!(conn, "a viewer let its queue grow past the control limit; disconnected");
                mailbox.close();
                return false;
            }
            send(out, mailbox, message)
        })
    }

    /// Sends the end of a transfer the viewer `conn` started (a download it receives is gone) to it alone, whatever its
    /// queue holds: it waits on it and may ask for nothing more, so it is never dropped, and since its queue can pass
    /// [`CONTROL_LIMIT`] without it asking (a costly refresh on top of notices), it never disconnects the viewer either.
    /// There is one per transfer the viewer started, so nobody else can make more of them.
    pub fn send_end_to(&self, conn: u64, message: &Message) -> bool {
        self.send_configured(conn, |mailbox, out| send(out, mailbox, message))
    }

    /// Sends a small message the viewer `conn` did not ask for (a download offered, or gone before anyone received it,
    /// a file chooser opened or closed: what the browser's launcher starts) to it alone, unless [`NOTICE_BUDGET`] bytes
    /// already wait for it; false when it did not go. It never disconnects the viewer, so the launcher cannot end a
    /// holder's connection, and with it the lease, by sending notices faster than the person's link takes them.
    pub fn send_notice_to(&self, conn: u64, message: &Message) -> bool {
        self.send_configured(conn, |mailbox, out| {
            mailbox.queued.load(Ordering::Relaxed) <= NOTICE_BUDGET && send(out, mailbox, message)
        })
    }

    /// Sends `message` to the viewer `conn` alone, unless [`SIDE_BUDGET`] bytes already wait for it; false when it did
    /// not go.
    pub fn send_to(&self, conn: u64, message: &Message) -> bool {
        self.send_configured(conn, |mailbox, out| {
            mailbox.queued.load(Ordering::Relaxed) <= SIDE_BUDGET && send(out, mailbox, message)
        })
    }
}

#[cfg(test)]
impl Viewers {
    /// A viewer `conn` that never reads, and a way to take what was queued for it since the last time.
    pub(crate) fn never_reading(&self, conn: u64) -> impl FnMut() -> Vec<Message> {
        let mailbox = Arc::new(Mailbox::new(64, 64));
        mailbox.configured.store(true, Ordering::Relaxed);
        let (out, queued) = mpsc::channel::<Out>();
        self.add(conn, mailbox, out);
        move || {
            let mut frames = FrameReader::new(Direction::ServerToClient);
            let mut messages = Vec::new();
            for Out::Sound(frame) | Out::Now(frame) | Out::Bulk(frame) in queued.try_iter() {
                frames.push(&frame).unwrap();
                while let Some(body) = frames.next_body().unwrap() {
                    match Link::decode(&body, Direction::ServerToClient).unwrap() {
                        Link::Relay(message) => messages.push(message),
                        _ => panic!("a link control"),
                    }
                }
            }
            messages
        }
    }
}

/// Starts the threads of one viewer connection, registered in `viewers` for out-of-band messages.
pub fn spawn(
    stream: UnixStream,
    conn: u64,
    mailbox: Arc<Mailbox>,
    events: calloop::channel::Sender<Event>,
    viewers: &Viewers,
) {
    let (to_writer, from_encoder) = mpsc::channel::<Out>();
    let Ok(write_half) = stream.try_clone() else { return };
    viewers.add(conn, mailbox.clone(), to_writer.clone());
    let writer_mailbox = mailbox.clone();
    let _ = std::thread::Builder::new()
        .name(format!("viewer-{conn}-write"))
        .spawn(move || writer(write_half, from_encoder, writer_mailbox));
    let encoder_mailbox = mailbox.clone();
    let encoder_out = to_writer.clone();
    let _ = std::thread::Builder::new()
        .name(format!("viewer-{conn}-encode"))
        .spawn(move || Encoder::new(encoder_mailbox, encoder_out).run());
    let _ = std::thread::Builder::new()
        .name(format!("viewer-{conn}-read"))
        .spawn(move || reader(stream, conn, mailbox, to_writer, events));
}

/// What a viewer's writer sends: sound first, then media and control as they come, and pieces of files only while
/// nothing else waits.
pub(super) enum Out {
    Sound(Vec<u8>),
    Now(Vec<u8>),
    Bulk(Vec<u8>),
}

/// A viewer's writer queue.
pub(super) type Outbox = mpsc::Sender<Out>;

pub(super) fn send(out: &Outbox, mailbox: &Mailbox, message: &Message) -> bool {
    let frame = Link::relay(message);
    mailbox.queued.fetch_add(frame.len(), Ordering::Relaxed);
    out.send(Out::Now(frame)).is_ok()
}

/// Queues a piece of a file behind everything else this viewer is sent.
pub(super) fn send_bulk(out: &Outbox, mailbox: &Mailbox, message: &Message) -> bool {
    let frame = Link::relay(message);
    mailbox.bulk_queued.fetch_add(frame.len(), Ordering::Relaxed);
    out.send(Out::Bulk(frame)).is_ok()
}

/// Frames taken from the writer's queue, by lane.
#[derive(Default)]
struct Lanes {
    sound: VecDeque<Vec<u8>>,
    now: VecDeque<Vec<u8>>,
    bulk: VecDeque<Vec<u8>>,
}

/// Messages taken from the queue in one pass before the writer writes again, so a busy queue never holds it back.
const MAX_DRAIN: usize = 64;

impl Lanes {
    fn take(&mut self, out: Out) {
        match out {
            Out::Sound(frame) => self.sound.push_back(frame),
            Out::Now(frame) => self.now.push_back(frame),
            Out::Bulk(frame) => self.bulk.push_back(frame),
        }
    }

    /// The next frame to write and its lane: sound first, then media and control in order, then a piece of a file;
    /// it waits for one when none is queued, and is `None` once the queue or the mailbox closed (an encoder that gave
    /// up closes the mailbox while others still hold senders to the queue).
    fn next(&mut self, frames: &mpsc::Receiver<Out>, mailbox: &Mailbox) -> Option<(Vec<u8>, Lane)> {
        loop {
            for out in frames.try_iter().take(MAX_DRAIN) {
                self.take(out);
            }
            if let Some(frame) = self.sound.pop_front() {
                return Some((frame, Lane::Sound));
            }
            if let Some(frame) = self.now.pop_front() {
                return Some((frame, Lane::Now));
            }
            if let Some(frame) = self.bulk.pop_front() {
                return Some((frame, Lane::Bulk));
            }
            match frames.recv_timeout(CLOSED_CHECK) {
                Ok(out) => self.take(out),
                Err(mpsc::RecvTimeoutError::Timeout) if !mailbox.is_closed() => {}
                Err(_) => return None,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane {
    Sound,
    Now,
    Bulk,
}

fn writer(mut stream: UnixStream, frames: mpsc::Receiver<Out>, mailbox: Arc<Mailbox>) {
    let _ = stream.set_write_timeout(Some(SEND_DEADLINE));
    let mut lanes = Lanes::default();
    while let Some((frame, lane)) = lanes.next(&frames, &mailbox) {
        if stream.write_all(&frame).is_err() {
            break;
        }
        let counter = match lane {
            Lane::Sound => &mailbox.sound_queued,
            Lane::Now => &mailbox.queued,
            Lane::Bulk => &mailbox.bulk_queued,
        };
        counter.fetch_sub(frame.len(), Ordering::Relaxed);
        mailbox.wake.notify_one();
        if mailbox.is_closed() {
            break;
        }
    }
    mailbox.close();
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn reader(
    mut stream: UnixStream,
    conn: u64,
    mailbox: Arc<Mailbox>,
    out: Outbox,
    events: calloop::channel::Sender<Event>,
) {
    let mut frames = FrameReader::new(Direction::ClientToServer);
    let mut chunk = vec![0u8; MAX_CHUNK];
    let mut attached = false;
    let _ = stream.set_read_timeout(Some(ATTACH_DEADLINE));
    let started = Instant::now();
    'read: loop {
        let n = match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if frames.push(&chunk[..n]).is_err() {
            break;
        }
        loop {
            let body = match frames.next_body() {
                Ok(Some(body)) => body,
                Ok(None) => break,
                Err(_) => break 'read,
            };
            let Ok(link) = Link::decode(&body, Direction::ClientToServer) else { break 'read };
            match (attached, link) {
                (false, Link::Attach { viewer }) => {
                    attached = true;
                    let _ = stream.set_read_timeout(None);
                    tracing::info!(conn, viewer, "viewer attached");
                }
                (false, _) => break 'read,
                (true, Link::Attach { .. }) => break 'read,
                (true, Link::Grant { lease_epoch, holder }) => {
                    let _ = events.send(Event::Grant { conn, epoch: lease_epoch, holder });
                }
                (true, Link::Settings { codec, bitrate_kbps, max_fps }) => {
                    let mut pending = mailbox.pending.lock().unwrap();
                    pending.settings = Settings { codec, bitrate_kbps, max_fps };
                    pending.settings_changed = true;
                    drop(pending);
                    mailbox.wake.notify_one();
                }
                (true, Link::OpenPage { request, lease_epoch, url }) => {
                    let _ = events.send(Event::OpenPage { conn, lease_epoch, request, url });
                }
                (true, Link::PageOpened { .. }) => break 'read,
                (true, Link::Relay(message)) => {
                    if !relay(message, conn, &mailbox, &out, &events) {
                        break 'read;
                    }
                }
            }
        }
        if !attached && started.elapsed() > ATTACH_DEADLINE {
            break;
        }
    }
    mailbox.close();
    let _ = events.send(Event::Closed { conn });
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

/// Handles one relayed client message; false closes the connection.
fn relay(
    message: Message,
    conn: u64,
    mailbox: &Mailbox,
    out: &Outbox,
    events: &calloop::channel::Sender<Event>,
) -> bool {
    match message {
        // The decoder already refused a hello of any version but VERSION, as it does a configuration.
        Message::Hello { codecs, .. } => {
            let mut pending = mailbox.pending.lock().unwrap();
            if pending.hello.is_some() {
                return false;
            }
            pending.hello = Some(codecs);
            drop(pending);
            mailbox.wake.notify_one();
            true
        }
        Message::KeyframeRequest { epoch } => {
            let mut pending = mailbox.pending.lock().unwrap();
            if epoch != mailbox.epoch.load(Ordering::Relaxed) {
                return true;
            }
            pending.refresh = true;
            drop(pending);
            mailbox.wake.notify_one();
            true
        }
        Message::ClientStats { presented, .. } => {
            mailbox.client_presented.store(presented.min(NO_REPORT - 1), Ordering::Relaxed);
            true
        }
        Message::TimePing { client_time_us } => {
            let server_time_us = super::clock_us();
            send(out, mailbox, &Message::TimePong { client_time_us, server_time_us })
        }
        message if message.is_input() => events.send(Event::Input { conn, message }).is_ok(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound(timestamp_us: u64) -> Message {
        Message::Audio { timestamp_us, payload: vec![0xfc; 100] }
    }

    /// Viewers with one viewer `conn`, configured, its mailbox and the end its writer would read.
    fn one_viewer(conn: u64) -> (Viewers, Arc<Mailbox>, Outbox, mpsc::Receiver<Out>) {
        let viewers = Viewers::default();
        let mailbox = Arc::new(Mailbox::new(64, 64));
        mailbox.configured.store(true, Ordering::Relaxed);
        let (out, frames) = mpsc::channel::<Out>();
        viewers.add(conn, mailbox.clone(), out.clone());
        (viewers, mailbox, out, frames)
    }

    /// Notices a viewer did not ask for, however many come while it reads nothing, are dropped before its queue nears
    /// the control limit: they never disconnect it, and its own replies still go.
    #[test]
    fn notices_never_disconnect_a_viewer_nor_crowd_out_its_replies() {
        let (viewers, mailbox, _out, _unread) = one_viewer(7);
        let notice = Message::FileRequestEnd { lease_epoch: 1, request: 1 };
        let mut sent = 0;
        while viewers.send_notice_to(7, &notice) {
            sent += 1;
        }
        assert!(sent > 1000 && !mailbox.is_closed(), "{sent} notices went, then they were dropped");
        assert!(mailbox.queued.load(Ordering::Relaxed) <= NOTICE_BUDGET + 64);
        let reply = Message::UploadAck { lease_epoch: 1, upload: 1, received: 0 };
        assert!(viewers.send_control_to(7, &reply), "the viewer's own reply goes");
        assert!(!mailbox.is_closed());
    }

    /// However much waits for the viewer, past the control limit even, the end of its own transfer goes and leaves it
    /// connected.
    #[test]
    fn a_transfers_end_goes_whatever_waits_and_never_disconnects() {
        let (viewers, mailbox, _out, _unread) = one_viewer(7);
        mailbox.queued.store(CONTROL_LIMIT + 1, Ordering::Relaxed);
        assert!(viewers.send_end_to(7, &Message::FileGone { lease_epoch: 1, file: 1 }));
        assert!(!mailbox.is_closed());
    }

    #[test]
    fn an_idle_writer_stops_once_its_mailbox_closed_though_senders_remain() {
        let mailbox = Mailbox::new(64, 64);
        let (_out, frames) = mpsc::channel::<Out>();
        mailbox.close();
        let started = Instant::now();
        assert!(Lanes::default().next(&frames, &mailbox).is_none());
        assert!(started.elapsed() < CLOSED_CHECK * 3);
    }

    #[test]
    fn sound_goes_out_ahead_of_media_however_much_media_waits() {
        let (viewers, mailbox, out, frames) = one_viewer(1);
        // A large refresh waits: more media than any side budget.
        let media = vec![1u8; QUEUE_BUDGET];
        mailbox.queued.fetch_add(media.len(), Ordering::Relaxed);
        out.send(Out::Now(media.clone())).unwrap();
        out.send(Out::Bulk(vec![2u8; 10])).unwrap();
        out.send(Out::Now(vec![3u8; 10])).unwrap();
        viewers.broadcast_sound(&sound(0));
        viewers.broadcast_sound(&sound(20_000));
        let queued = mailbox.sound_queued.load(Ordering::Relaxed);
        assert!(queued > 0, "sound was admitted behind {} bytes of media", media.len());
        let mut lanes = Lanes::default();
        let order: Vec<(Lane, usize)> =
            (0..5).map(|_| lanes.next(&frames, &mailbox).map(|(f, l)| (l, f.len())).unwrap()).collect();
        let first = Link::Relay(sound(0)).encode().len();
        assert_eq!(
            order,
            [
                (Lane::Sound, first),
                (Lane::Sound, queued - first),
                (Lane::Now, media.len()),
                (Lane::Now, 10),
                (Lane::Bulk, 10)
            ]
        );
    }

    #[test]
    fn sound_waiting_for_a_viewer_that_stopped_reading_is_bounded() {
        let viewers = Viewers::default();
        let mailbox = Arc::new(Mailbox::new(64, 64));
        let (out, frames) = mpsc::channel();
        viewers.add(1, mailbox.clone(), out);
        // Before its configuration, a viewer gets no sound.
        viewers.broadcast_sound(&sound(0));
        assert_eq!(mailbox.sound_queued.load(Ordering::Relaxed), 0);
        mailbox.configured.store(true, Ordering::Relaxed);
        for i in 0..1000 {
            viewers.broadcast_sound(&sound(i * 20_000));
        }
        let queued = mailbox.sound_queued.load(Ordering::Relaxed);
        assert!(queued <= SOUND_BUDGET && queued > SOUND_BUDGET / 2, "{queued} bytes of sound queued");
        assert_eq!(frames.try_iter().map(|o| if let Out::Sound(f) = o { f.len() } else { 0 }).sum::<usize>(), queued);
        // A viewer whose writer is gone: nothing stays counted.
        drop(frames);
        mailbox.sound_queued.store(0, Ordering::Relaxed);
        viewers.broadcast_sound(&sound(0));
        assert_eq!(mailbox.sound_queued.load(Ordering::Relaxed), 0);
    }
}
