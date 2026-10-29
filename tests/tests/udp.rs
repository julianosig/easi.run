//! The direct path (admin/src/udp.rs), on in the bench stack: a viewer that opens the admin's WebTransport session gets
//! its media and sound there, its acknowledgements and keyframe requests go back over it, the media come back on the
//! WebSocket in a new epoch when the session goes, and the port admits nothing but a live ticket from the admin's own
//! origin.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Admin, Seen, Viewer, origin, uncovered};
use display_protocol::frame::{FrameReader, Link};
use display_protocol::{Direction, Message, Rect};
use wtransport::endpoint::ConnectOptions;
use wtransport::error::ConnectingError;
use wtransport::tls::Sha256Digest;
use wtransport::{ClientConfig, Connection, Endpoint};

const ARTICLE: &str = "https://fixture.bench/article.html";
const TONE: &str = "https://fixture.bench/tone.html";

/// The viewer's next offer: its generation, certificate hash, and address (which carries the ticket).
async fn offer(viewer: &Viewer) -> (u32, [u8; 32], String) {
    let before = viewer.seen.lock().unwrap().udp_offers.len();
    viewer.send(Message::UdpRequest).await;
    viewer.wait_for("an offer", |seen| seen.udp_offers.get(before).cloned()).await
}

/// Opens an offer's address, with `origin` as its Origin, the certificate pinned to `hash`. The bench stack's address
/// names localhost, which may resolve to IPv6 first; its port is published on IPv4.
async fn open(url: &str, hash: [u8; 32], origin: Option<&str>) -> Result<Connection, ConnectingError> {
    let config =
        ClientConfig::builder().with_bind_default().with_server_certificate_hashes([Sha256Digest::new(hash)]).build();
    let endpoint = Endpoint::client(config).expect("a client endpoint");
    let mut options = ConnectOptions::builder(url.replace("//localhost:", "//127.0.0.1:"));
    if let Some(origin) = origin {
        options = options.add_header("origin", origin);
    }
    endpoint.connect(options.build()).await
}

/// The rectangles of the tiles in `log`.
fn tiles_in(log: &[(Instant, Message)]) -> impl Iterator<Item = &Rect> {
    log.iter().flat_map(|(_, m)| match m {
        Message::Tiles { rects, .. } => &rects[..],
        _ => &[],
    })
}

/// A viewer's session, as the web client uses it: what comes over it goes into the viewer's log (counted as direct),
/// and every video, tiles and cursor message is acknowledged over it.
struct Direct {
    connection: Connection,
    upstream: tokio::sync::mpsc::Sender<Message>,
    /// The messages that came as datagrams.
    datagrams: Arc<AtomicUsize>,
}

impl Direct {
    async fn start(connection: Connection, seen: Arc<Mutex<Seen>>) -> Direct {
        let (upstream, mut outgoing) = tokio::sync::mpsc::channel::<Message>(256);
        let mut stream = connection.open_uni().await.expect("a stream").await.expect("a stream opened");
        tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                if stream.write_all(&Link::Relay(message).encode()).await.is_err() {
                    break;
                }
            }
        });
        let (media, acks, sound) = (connection.clone(), upstream.clone(), connection.clone());
        let log = seen.clone();
        let datagrams = Arc::new(AtomicUsize::new(0));
        let counted = datagrams.clone();
        tokio::spawn(async move {
            let Ok(mut stream) = media.accept_uni().await else { return };
            let mut frames = FrameReader::new(Direction::ServerToClient);
            let mut chunk = vec![0u8; display_protocol::frame::MAX_CHUNK];
            while let Ok(Some(n)) = stream.read(&mut chunk).await {
                frames.push(&chunk[..n]).expect("a frame");
                while let Some(body) = frames.next_body().expect("a frame") {
                    let Link::Relay(message) = Link::decode(&body, Direction::ServerToClient).expect("a message")
                    else {
                        panic!("link control on the media stream")
                    };
                    if let Some(revision) = message.media_revision() {
                        let _ = acks.send(Message::MediaAck { revision }).await;
                    }
                    let mut seen = log.lock().unwrap();
                    seen.direct += 1;
                    seen.record(message);
                }
            }
        });
        tokio::spawn(async move {
            while let Ok(datagram) = sound.receive_datagram().await {
                let message = Message::decode(&datagram, Direction::ServerToClient).expect("a message");
                assert!(matches!(message, Message::Audio { .. }), "{message:?} in a datagram");
                let mut seen = seen.lock().unwrap();
                // Under the log's lock, so the count and the log always agree.
                counted.fetch_add(1, Ordering::Relaxed);
                seen.direct += 1;
                seen.record(message);
            }
        });
        Direct { connection, upstream, datagrams }
    }

    async fn send(&self, message: Message) {
        self.upstream.send(message).await.expect("the session is open");
    }
}

/// Moves `viewer`'s media onto a session of its own, once its first picture came over the WebSocket.
async fn moved(viewer: &Viewer) -> (Direct, usize) {
    viewer
        .wait_for("the first picture", |seen| {
            seen.log.iter().any(|(_, m)| matches!(m, Message::Tiles { .. })).then_some(())
        })
        .await;
    let (generation, hash, url) = offer(viewer).await;
    let connection = open(&url, hash, Some(&origin())).await.expect("the session");
    let direct = Direct::start(connection, viewer.seen.clone()).await;
    let at =
        viewer.wait_for("media-moved", |seen| seen.moved.iter().find(|(g, _)| *g == generation).map(|m| m.1)).await;
    (direct, at)
}

fn epoch_of(log: &[(std::time::Instant, Message)]) -> u32 {
    log.iter().rev().find_map(|(_, m)| if let Message::Config { epoch, .. } = m { Some(*epoch) } else { None }).unwrap()
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn media_take_the_direct_path_and_its_keyframe_requests_are_answered_there() {
    let admin = Admin::login().await;
    let session = admin.create(ARTICLE).await;
    let viewer = admin.watch(&session, 0).await;
    let (direct, at) = moved(&viewer).await;
    let epoch = epoch_of(&viewer.log());
    // A refresh asked for over the session comes over it, whole, and nothing more comes on the WebSocket. The page has
    // settled first, so what comes after the request is its answer, not the page still painting.
    let tiles = |seen: &[(std::time::Instant, Message)]| {
        seen.iter().filter(|(_, m)| matches!(m, Message::Tiles { .. })).count()
    };
    let mut settled = tiles(&viewer.seen.lock().unwrap().log[at..]);
    loop {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let now = tiles(&viewer.seen.lock().unwrap().log[at..]);
        if now == settled {
            break;
        }
        settled = now;
    }
    let (width, height) = {
        let seen = viewer.seen.lock().unwrap();
        seen.log
            .iter()
            .rev()
            .find_map(
                |(_, m)| if let Message::Config { width, height, .. } = m { Some((*width, *height)) } else { None },
            )
            .unwrap()
    };
    let asked = viewer.seen.lock().unwrap().log.len();
    direct.send(Message::KeyframeRequest { epoch }).await;
    // The answer covers every pixel of the screen.
    let screen = Rect { x: 0, y: 0, w: width, h: height };
    viewer
        .wait_for("the whole refresh over the session", |seen| {
            (seen.direct > 0 && uncovered(&screen, tiles_in(&seen.log[asked..])) == 0).then_some(())
        })
        .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    {
        let seen = viewer.seen.lock().unwrap();
        let media = seen.log[at..].iter().filter(|(_, m)| !matches!(m, Message::Audio { .. })).count();
        assert_eq!(media as u32, seen.direct, "every media message after the move came over the session");
        assert!(seen.fell_back.is_empty(), "the path held");
    }
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn sound_comes_over_the_direct_path_as_datagrams() {
    let admin = Admin::login().await;
    let (session, viewer, epoch) = admin.hold(TONE, "Tone").await;
    let (direct, at) = moved(&viewer).await;
    let sound = |seen: &Seen| seen.log[at..].iter().filter(|(_, m)| matches!(m, Message::Audio { .. })).count();
    // A page may start sound only after a gesture: clicked until it plays (its tone lasts 1.5 s).
    for _ in 0..5 {
        viewer.click(epoch, 640, 400).await;
        tokio::time::sleep(Duration::from_secs(3)).await;
        if sound(&viewer.seen.lock().unwrap()) >= 25 {
            break;
        }
    }
    {
        let seen = viewer.seen.lock().unwrap();
        assert!(sound(&seen) >= 25, "{} packets of sound", sound(&seen));
        let media = seen.log[at..].len();
        assert_eq!(seen.direct as usize, media, "all of it, and the media, over the session");
        assert_eq!(direct.datagrams.load(Ordering::Relaxed), sound(&seen), "the sound, and only it, as datagrams");
    }
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_lost_session_brings_the_media_back_on_the_websocket_in_a_new_epoch() {
    let admin = Admin::login().await;
    let session = admin.create(ARTICLE).await;
    let viewer = admin.watch(&session, 0).await;
    let (direct, _) = moved(&viewer).await;
    let epoch = epoch_of(&viewer.log());
    direct.connection.close(0u32.into(), b"gone");
    let at = viewer.wait_for("media-fallback", |seen| seen.fell_back.first().map(|f| f.1)).await;
    let direct = viewer.seen.lock().unwrap().direct;
    // After the fallback: a configuration of a newer epoch first, then its full refresh, all on the WebSocket.
    viewer
        .wait_for("a new epoch on the WebSocket", |seen| {
            let after = &seen.log[at..];
            let config = after.iter().position(|(_, m)| matches!(m, Message::Config { epoch: e, .. } if *e > epoch))?;
            let Message::Config { width, height, .. } = after[config].1 else { unreachable!() };
            let screen = Rect { x: 0, y: 0, w: width, h: height };
            (uncovered(&screen, tiles_in(&after[config..])) == 0).then_some(())
        })
        .await;
    {
        let seen = viewer.seen.lock().unwrap();
        assert_eq!(seen.direct, direct, "nothing more came over the closed session");
        let first = seen.log[at..].iter().find(|(_, m)| !matches!(m, Message::Audio { .. }));
        assert!(matches!(first, Some((_, Message::Config { .. })) | None), "media before the new epoch: {first:?}");
    }
    // A new offer (no sooner than 10 s after the last) moves them again.
    tokio::time::sleep(Duration::from_secs(10)).await;
    let (generation, hash, url) = offer(&viewer).await;
    let connection = open(&url, hash, Some(&origin())).await.expect("the second session");
    let _again = Direct::start(connection, viewer.seen.clone()).await;
    viewer.wait_for("the second move", |seen| seen.moved.iter().any(|(g, _)| *g == generation).then_some(())).await;
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn the_port_admits_only_a_live_ticket_from_the_admins_origin() {
    let admin = Admin::login().await;
    let session = admin.create(ARTICLE).await;
    let refused = |result: Result<Connection, ConnectingError>, what: &str| match result {
        Err(ConnectingError::SessionRejected) => {}
        other => panic!("{what}: {:?}", other.map(|_| "admitted")),
    };
    // One viewer per offer: offers are spaced out per viewer.
    let viewer = admin.watch(&session, 0).await;
    let (_, hash, url) = offer(&viewer).await;
    let base = url.split("/media?").next().unwrap().to_owned();
    // No ticket, a ticket never issued, and another path.
    refused(open(&format!("{base}/media"), hash, Some(&origin())).await, "no ticket");
    refused(open(&format!("{base}/media?t={}", "ab".repeat(32)), hash, Some(&origin())).await, "a ticket never issued");
    refused(open(&url.replace("/media?", "/other?"), hash, Some(&origin())).await, "another path");
    // The right ticket from another origin is refused, and spent.
    refused(open(&url, hash, Some("https://elsewhere.example")).await, "another origin");
    refused(open(&url, hash, Some(&origin())).await, "a ticket spent by a refused attempt");
    // A fresh one is taken once.
    let viewer = admin.watch(&session, 0).await;
    let (_, hash, url) = offer(&viewer).await;
    let admitted = open(&url, hash, Some(&origin())).await.expect("a live ticket");
    refused(open(&url, hash, Some(&origin())).await, "a ticket taken");
    drop(admitted);
    // One whose viewer left is refused.
    let viewer = admin.watch(&session, 0).await;
    let (_, hash, url) = offer(&viewer).await;
    viewer.close().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    refused(open(&url, hash, Some(&origin())).await, "the ticket of a viewer that left");
    // Pinned to another certificate, the client itself refuses the server.
    let viewer = admin.watch(&session, 0).await;
    let (_, _, url) = offer(&viewer).await;
    assert!(open(&url, [7; 32], Some(&origin())).await.is_err(), "a session with an unpinned certificate");
    admin.stop(&session).await;
}

#[tokio::test]
#[ignore = "needs the bench stack: scripts/e2e.sh"]
async fn a_connection_that_never_asks_is_closed_at_its_deadline() {
    let admin = Admin::login().await;
    let session = admin.create(ARTICLE).await;
    let viewer = admin.watch(&session, 0).await;
    let (_, hash, url) = offer(&viewer).await;
    let port: u16 = url.split(':').nth(2).and_then(|p| p.split('/').next()).unwrap().parse().unwrap();
    let config =
        ClientConfig::builder().with_bind_default().with_server_certificate_hashes([Sha256Digest::new(hash)]).build();
    let mut endpoint = wtransport::quinn::Endpoint::client("0.0.0.0:0".parse().unwrap()).expect("a QUIC endpoint");
    endpoint.set_default_client_config(config.quic_config().clone());
    let connection = endpoint
        .connect(([127, 0, 0, 1], port).into(), "localhost")
        .expect("a connection")
        .await
        .expect("the QUIC handshake");
    // It says nothing more, no HTTP/3 request, while the admin's keepalives keep it busy: the admin closes it at its
    // deadline (5 s from its first packet).
    let started = std::time::Instant::now();
    let closed = tokio::time::timeout(Duration::from_secs(10), connection.closed()).await;
    assert!(closed.is_ok(), "an unadmitted connection still open after 10 s");
    assert!(started.elapsed() < Duration::from_secs(7), "closed after {:?}", started.elapsed());
    admin.stop(&session).await;
}
