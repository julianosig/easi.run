//! The direct media path: a WebTransport session (HTTP/3 over QUIC) beside a viewer's WebSocket, which carries its
//! media where UDP reaches the server (see the protocol crate's docs, and `relay` for what goes over it).
//!
//! On when `ADMIN_UDP_URL` names where viewers reach the admin's UDP port. A viewer's relay hands it an offer over its
//! signed-in WebSocket: the address, which carries a ticket, and the SHA-256 of this endpoint's certificate. A ticket
//! is 32 random bytes, valid once and for [`TICKET_LIFE`], for that viewer of that session alone. The endpoint admits a
//! session only for a ticket it redeems while that viewer is still connected and its login live, from the admin's own
//! origin, and hands it to that viewer's relay; anything else gets a 404 and its connection closes.
//!
//! Before a ticket, what anyone can reach: QUIC's handshake (an address it has not validated gets a stateless Retry
//! first, so a spoofed one gets one packet and no state), TLS 1.3 with this endpoint's self-signed certificate, and
//! HTTP/3's setup and requests. At most [`MAX_HANDSHAKES`] attempts are under way at once (others are ignored, as are
//! packets beyond quinn's [`MAX_INCOMING`] pending connections), at most [`MAX_CONNECTIONS`] connections are open,
//! and each attempt has [`ADMISSION`] from its validated first packet to being admitted, however busy it keeps its
//! connection: then the admin closes it (the patched wtransport takes a connection the admin keeps a handle to). No
//! 0-RTT. The certificate (ECDSA P-256) is valid for 13 days and an hour, within the
//! two weeks a browser accepts for a certificate it is given the hash of, generated in memory at startup and replaced
//! every [`ROTATE_EVERY`]; connections keep the one they started with.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use display_protocol::frame::{FrameReader, Link};
use display_protocol::{Direction, Message};
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use wtransport::endpoint::IncomingSessionFuture;
use wtransport::quinn;
use wtransport::quinn::congestion::BbrConfig;
use wtransport::tls::{Certificate, CertificateChain, PrivateKey};
use wtransport::{Identity, ServerConfig};

use crate::app::{App, ToViewer};
use crate::auth::{hex, unhex};

/// Where the admin's container listens: compose publishes it.
pub const LISTEN: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 9443);
/// How long a ticket may wait to be redeemed.
pub const TICKET_LIFE: Duration = Duration::from_secs(10);
/// Tickets outstanding at once, across every viewer (each viewer has at most one): the oldest goes first.
const MAX_TICKETS: usize = 64;
/// From a connection's first packet to its admission (QUIC, TLS, HTTP/3 and the ticket), at most.
pub const ADMISSION: Duration = Duration::from_secs(5);
/// Handshakes under way at once.
pub const MAX_HANDSHAKES: usize = 8;
/// Connections quinn holds before the endpoint looks at them, and the bytes each, and all of them, may buffer.
const MAX_INCOMING: usize = 16;
/// Connections open at once, admitted or not: past it, a new one is refused.
const MAX_CONNECTIONS: usize = 64;
/// What a client may send ahead on one stream (and four times that on all of them): acknowledgements and requests.
const RECEIVE_WINDOW: u32 = 64 * 1024;
const INCOMING_BUFFER: u64 = 64 * 1024;
/// How often the certificate is replaced, how long before its making it is valid from (clocks differ), and for how
/// long after.
const ROTATE_EVERY: Duration = Duration::from_secs(6 * 24 * 3600);
const BACKDATE: time::Duration = time::Duration::hours(1);
const VALID_FOR: time::Duration = time::Duration::days(13);
/// A connection with nothing on it for this long is gone; the endpoint keeps an idle one alive more often than that.
const IDLE_TIMEOUT: Duration = Duration::from_secs(6);
const KEEP_ALIVE: Duration = Duration::from_secs(2);
/// The path of the session's address, before its ticket.
const PATH: &str = "/media?t=";

/// The direct path's state: where viewers reach it, its tickets, and the hash of the certificate it serves.
pub struct Udp {
    /// `https://host:port`, from `ADMIN_UDP_URL`.
    origin: String,
    tickets: Mutex<HashMap<[u8; 32], Ticket>>,
    cert_sha256: Mutex<[u8; 32]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub session: String,
    pub viewer: u64,
    pub generation: u32,
    issued: Instant,
}

impl Udp {
    pub fn new(origin: String) -> Udp {
        Udp { origin, tickets: Mutex::default(), cert_sha256: Mutex::new([0; 32]) }
    }

    /// The origin viewers open the session at, which the page's content security policy lets it reach.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// An offer for `viewer` of `session`, with a new ticket, which replaces any it had.
    pub fn offer(&self, session: &str, viewer: u64, generation: u32) -> Message {
        let mut ticket = [0u8; 32];
        rand::rng().fill_bytes(&mut ticket);
        let mut tickets = self.tickets.lock().unwrap();
        tickets.retain(|_, t| t.issued.elapsed() < TICKET_LIFE && !(t.session == session && t.viewer == viewer));
        while tickets.len() >= MAX_TICKETS {
            let oldest = tickets.iter().min_by_key(|(_, t)| t.issued).map(|(k, _)| *k).expect("not empty");
            tickets.remove(&oldest);
        }
        let issued = Instant::now();
        tickets.insert(ticket, Ticket { session: session.to_owned(), viewer, generation, issued });
        drop(tickets);
        let cert_sha256 = *self.cert_sha256.lock().unwrap();
        Message::UdpOffer { generation, cert_sha256, url: format!("{}{PATH}{}", self.origin, hex(&ticket)) }
    }

    /// The ticket a session request's path carries, taken (whatever follows), if it is one still valid.
    pub fn redeem(&self, path: &str) -> Option<Ticket> {
        let ticket = unhex(path.strip_prefix(PATH)?)?;
        let taken = self.tickets.lock().unwrap().remove(&ticket)?;
        (taken.issued.elapsed() < TICKET_LIFE).then_some(taken)
    }

    /// Drops the tickets of a viewer that left.
    pub fn forget(&self, session: &str, viewer: u64) {
        self.tickets.lock().unwrap().retain(|_, t| !(t.session == session && t.viewer == viewer));
    }
}

/// A new certificate (ECDSA P-256, self-signed) and the SHA-256 of its DER.
fn certificate() -> Result<(Identity, [u8; 32]), String> {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| e.to_string())?;
    let mut params = rcgen::CertificateParams::new(vec!["easi.run".to_owned()]).map_err(|e| e.to_string())?;
    params.distinguished_name.push(rcgen::DnType::CommonName, "easi.run");
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - BACKDATE;
    params.not_after = now + VALID_FOR;
    let cert = params.self_signed(&key).map_err(|e| e.to_string())?;
    let sha256: [u8; 32] = Sha256::digest(cert.der()).into();
    let chain = CertificateChain::single(Certificate::from_der(cert.der().to_vec()).map_err(|e| e.to_string())?);
    Ok((Identity::new(chain, PrivateKey::from_der_pkcs8(key.serialize_der())), sha256))
}

fn server_config(identity: Identity) -> quinn::ServerConfig {
    let mut transport = quinn::TransportConfig::default();
    // BBR keeps the rate a lossy path can carry where loss-based control (CUBIC) collapses.
    transport.congestion_controller_factory(Arc::new(BbrConfig::default()));
    transport.max_idle_timeout(Some(IDLE_TIMEOUT.try_into().expect("a valid idle timeout")));
    transport.keep_alive_interval(Some(KEEP_ALIVE));
    // HTTP/3's own streams and the client's one stream, little room for what a client sends (its acknowledgements and
    // requests), and little for datagrams, which a client has no reason to send.
    transport.max_concurrent_bidi_streams(2u32.into());
    transport.max_concurrent_uni_streams(8u32.into());
    transport.stream_receive_window(RECEIVE_WINDOW.into());
    transport.receive_window((RECEIVE_WINDOW * 4).into());
    transport.datagram_receive_buffer_size(Some(16 * 1024));
    let config = ServerConfig::builder().with_bind_address(LISTEN).with_custom_transport(identity, transport).build();
    let mut quic = config.quic_config().clone();
    quic.max_incoming(MAX_INCOMING)
        .incoming_buffer_size(INCOMING_BUFFER)
        .incoming_buffer_size_total(INCOMING_BUFFER * MAX_INCOMING as u64);
    quic
}

/// The endpoint, bound now so a port that cannot be had stops the start; [`serve`] runs it.
pub fn endpoint(udp: &Udp) -> Result<quinn::Endpoint, String> {
    let (identity, sha256) = certificate()?;
    let endpoint =
        quinn::Endpoint::server(server_config(identity), LISTEN).map_err(|e| format!("the UDP port {LISTEN}: {e}"))?;
    *udp.cert_sha256.lock().unwrap() = sha256;
    Ok(endpoint)
}

/// Admits sessions as long as the admin runs, and replaces the certificate every [`ROTATE_EVERY`].
pub async fn serve(app: Arc<App>, endpoint: quinn::Endpoint) {
    let handshakes = Arc::new(Semaphore::new(MAX_HANDSHAKES));
    let mut rotate = tokio::time::interval_at(tokio::time::Instant::now() + ROTATE_EVERY, ROTATE_EVERY);
    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { return };
                if endpoint.open_connections() >= MAX_CONNECTIONS {
                    incoming.refuse();
                    continue;
                }
                if !incoming.remote_address_validated() {
                    let _ = incoming.retry();
                    continue;
                }
                let Ok(permit) = handshakes.clone().try_acquire_owned() else {
                    incoming.ignore();
                    continue;
                };
                // Accepted here, beside the count, so attempts cannot take it past the bound.
                let Ok(connecting) = incoming.accept() else { continue };
                let app = app.clone();
                tokio::spawn(async move {
                    attempt(&app, connecting).await;
                    drop(permit);
                });
            }
            _ = rotate.tick() => match certificate() {
                Ok((identity, sha256)) => {
                    endpoint.set_server_config(Some(server_config(identity)));
                    let udp = app.udp.as_ref().expect("served only with the direct path on");
                    *udp.cert_sha256.lock().unwrap() = sha256;
                }
                Err(e) => tracing::warn!(%e, "the direct path's new certificate could not be made"),
            },
        }
    }
}

/// One attempt, from its validated first packet: its handshake and request, then its ticket, then the session to its
/// viewer's relay, all within [`ADMISSION`]. Refused or late, its connection is closed, whatever HTTP/3's own task
/// still holds of it.
async fn attempt(app: &App, connecting: quinn::Connecting) {
    let mut held: Option<quinn::Connection> = None;
    let admitted = tokio::time::timeout(ADMISSION, async {
        let connection = connecting.await.ok()?;
        held = Some(connection.clone());
        admit(app, connection).await
    })
    .await;
    if let Ok(Some((relay, generation, session))) = admitted
        && relay.try_send(ToViewer::Udp { generation, connection: session }).is_ok()
    {
        return;
    }
    if let Some(connection) = held {
        connection.close(0u32.into(), b"");
    }
}

/// The session request on `connection`, admitted for a live ticket of its viewer from the admin's origin, else
/// answered with a 404.
async fn admit(
    app: &App,
    connection: quinn::Connection,
) -> Option<(tokio::sync::mpsc::Sender<ToViewer>, u32, wtransport::Connection)> {
    let request = IncomingSessionFuture::with_quic_connection(connection).await.ok()?;
    let udp = app.udp.as_ref().expect("served only with the direct path on");
    let ticket = udp.redeem(request.path()).filter(|_| request.origin() == Some(app.config.origin.as_str()));
    let Some((ticket, relay)) = ticket.and_then(|t| app.udp_viewer(&t).map(|relay| (t, relay))) else {
        request.not_found().await;
        return None;
    };
    let session = request.accept().await.ok()?;
    tracing::info!(session = %ticket.session, viewer = ticket.viewer, "direct path admitted");
    Some((relay, ticket.generation, session))
}

/// How long media sent over the session may wait for its acknowledgement before the session counts as lost (the
/// display holds new media back long before: see its flight).
pub const STALL: Duration = Duration::from_secs(2);
/// Media messages waiting for the session's stream, at most: a stream that stopped taking them is lost.
const MEDIA_QUEUE: usize = 256;
/// The largest message the client's stream may carry: an acknowledgement or a keyframe request, with room to spare.
const MAX_UPSTREAM: usize = 64;

/// A viewer's admitted session, as its relay uses it: the media stream's writer, the client's stream's reader, and
/// the media sent and not acknowledged yet. Dropped, it closes the session.
pub struct Path {
    pub generation: u32,
    connection: wtransport::Connection,
    media: tokio::sync::mpsc::Sender<axum::body::Bytes>,
    /// The client's messages, until its stream or the session ends.
    pub upstream: tokio::sync::mpsc::Receiver<Message>,
    unacked: std::collections::VecDeque<(u64, Instant)>,
    tasks: [tokio::task::JoinHandle<()>; 2],
}

impl Path {
    pub fn open(generation: u32, connection: wtransport::Connection) -> Path {
        let (media, mut queued) = tokio::sync::mpsc::channel::<axum::body::Bytes>(MEDIA_QUEUE);
        let (to_relay, upstream) = tokio::sync::mpsc::channel(MEDIA_QUEUE);
        let writer = connection.clone();
        let writer = tokio::spawn(async move {
            let Ok(opening) = writer.open_uni().await else { return };
            let Ok(mut stream) = opening.await else { return };
            // Each message as on the display link: its length, then the link's relay tag and the message.
            while let Some(body) = queued.recv().await {
                let len = (body.len() as u32).to_le_bytes();
                if stream.write_all(&len).await.is_err() || stream.write_all(&body).await.is_err() {
                    return;
                }
            }
        });
        let reader = connection.clone();
        let reader = tokio::spawn(async move {
            // One stream from the client, from the start: a second one, a bidirectional one, or a datagram closes the
            // session.
            let (bi, datagram) = (reader.clone(), reader.clone());
            let guard = async move {
                tokio::select! {
                    _ = bi.accept_bi() => {}
                    _ = datagram.receive_datagram() => {}
                }
            };
            let read = async {
                let Ok(mut stream) = reader.accept_uni().await else { return };
                let second = reader.clone();
                let second = async move {
                    let _ = second.accept_uni().await;
                };
                tokio::pin!(second);
                let mut frames = FrameReader::new(Direction::ClientToServer);
                let mut chunk = vec![0u8; 4096];
                loop {
                    let n = tokio::select! {
                        read = stream.read(&mut chunk) => match read {
                            Ok(Some(n)) => n,
                            _ => return,
                        },
                        () = &mut second => return,
                    };
                    if frames.push(&chunk[..n]).is_err() {
                        return;
                    }
                    loop {
                        let body = match frames.next_body() {
                            Ok(Some(body)) => body,
                            Ok(None) => break,
                            Err(_) => return,
                        };
                        // Only these, small; anything else closes the session, as on the WebSocket.
                        let message = match Link::decode(&body, Direction::ClientToServer) {
                            Ok(Link::Relay(m @ (Message::MediaAck { .. } | Message::KeyframeRequest { .. })))
                                if body.len() <= MAX_UPSTREAM =>
                            {
                                m
                            }
                            _ => return,
                        };
                        if to_relay.send(message).await.is_err() {
                            return;
                        }
                    }
                }
            };
            tokio::select! { () = guard => {}, () = read => {} }
            reader.close(0u32.into(), b"");
        });
        Path { generation, connection, media, upstream, unacked: Default::default(), tasks: [writer, reader] }
    }

    /// Queues one display message (`body`: the link's relay tag and the message) on the media stream; `revision` is
    /// the one the client acknowledges it by. False once the session cannot take it.
    pub fn send_media(&mut self, revision: Option<u64>, body: axum::body::Bytes) -> bool {
        if self.media.try_send(body).is_err() {
            return false;
        }
        if let Some(revision) = revision {
            self.unacked.push_back((revision, Instant::now()));
        }
        true
    }

    /// Sends one sound message (`body` as for [`Path::send_media`]) as a datagram, or on the media stream when the
    /// path cannot carry it in one.
    pub fn send_sound(&mut self, body: axum::body::Bytes) -> bool {
        let message = &body[1..];
        if self.connection.max_datagram_size().is_some_and(|max| message.len() <= max) {
            // A datagram that finds no room is dropped, as sound lost on the way would be.
            let _ = self.connection.send_datagram(message);
            return true;
        }
        self.send_media(None, body)
    }

    /// The client acknowledged every media message up to `revision`.
    pub fn acked(&mut self, revision: u64) {
        while self.unacked.front().is_some_and(|(r, _)| *r <= revision) {
            self.unacked.pop_front();
        }
    }

    /// Whether media sent over the session has waited longer than [`STALL`] for its acknowledgement.
    pub fn stalled(&self) -> bool {
        self.unacked.front().is_some_and(|(_, at)| at.elapsed() > STALL)
    }
}

impl Drop for Path {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        self.connection.close(0u32.into(), b"");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_is_taken_once_by_its_own_path_and_only_while_young() {
        let udp = Udp::new("https://203.0.113.7:9443".into());
        let Message::UdpOffer { url, generation, .. } = udp.offer("abc", 7, 2) else { panic!("an offer") };
        assert_eq!(generation, 2);
        let path = url.strip_prefix("https://203.0.113.7:9443").unwrap().to_owned();
        assert!(path.starts_with(PATH) && path.len() == PATH.len() + 64, "{path}");
        // Another viewer's, a malformed one, or one never issued: none.
        assert_eq!(udp.redeem(&path.replace(PATH, "/other?t=")), None);
        assert_eq!(udp.redeem(&format!("{PATH}{}", "0".repeat(64))), None);
        assert_eq!(udp.redeem(&path[..path.len() - 1]), None);
        let ticket = udp.redeem(&path).expect("a valid ticket");
        assert_eq!((ticket.session.as_str(), ticket.viewer, ticket.generation), ("abc", 7, 2));
        assert_eq!(udp.redeem(&path), None, "taken once");
        // A newer offer to the same viewer replaces its earlier ticket; a viewer that left has none.
        let Message::UdpOffer { url: first, .. } = udp.offer("abc", 7, 3) else { panic!() };
        let Message::UdpOffer { url: second, .. } = udp.offer("abc", 7, 4) else { panic!() };
        let path = |url: &str| url.strip_prefix("https://203.0.113.7:9443").unwrap().to_owned();
        assert_eq!(udp.redeem(&path(&first)), None);
        udp.forget("abc", 7);
        assert_eq!(udp.redeem(&path(&second)), None);
        // An old one is refused.
        let Message::UdpOffer { url, .. } = udp.offer("abc", 8, 1) else { panic!() };
        udp.tickets.lock().unwrap().values_mut().for_each(|t| t.issued -= TICKET_LIFE);
        assert_eq!(udp.redeem(&path(&url)), None);
    }

    #[test]
    fn outstanding_tickets_are_bounded() {
        let udp = Udp::new("https://h:1".into());
        for viewer in 0..(MAX_TICKETS as u64 * 2) {
            udp.offer("abc", viewer, 1);
        }
        assert_eq!(udp.tickets.lock().unwrap().len(), MAX_TICKETS);
    }

    #[test]
    fn the_certificate_is_p256_and_valid_for_less_than_two_weeks() {
        let (identity, sha256) = certificate().unwrap();
        let der = identity.certificate_chain().as_slice()[0].der().to_vec();
        assert_eq!(<[u8; 32]>::from(Sha256::digest(&der)), sha256);
        // The ECDSA P-256 public key's object identifiers, and the validity's two times.
        let has = |needle: &[u8]| der.windows(needle.len()).any(|w| w == needle);
        assert!(has(&[0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]), "id-ecPublicKey");
        assert!(has(&[0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]), "prime256v1");
        assert!(BACKDATE + VALID_FOR < time::Duration::days(14));
    }
}
