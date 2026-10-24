//! The HTTPS server: each connection's TLS handshake, then HTTP/1.1, the one protocol its TLS identity offers (see
//! `tls`), WebSocket upgrades included. It bounds connections before any route or login check sees them: how many may
//! be open at once, how long the handshake and a request's headers may take, how much of a request's head is buffered,
//! and how long a write may wait for a client that reads nothing (see `deadline`). None of these cuts a connection
//! that waits on the admin: a request waiting for a long agent call, or an idle viewer stream.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use crate::deadline::{self, Deadline};

/// Connections in their handshake or served as HTTP at once; past them, a new one is closed as it comes. A browser
/// opens a handful per origin, an agent one per call in flight. A WebSocket upgrade leaves the count: a viewer's stream
/// is opened only for a live login.
const MAX_CONNECTIONS: usize = 256;
/// How long a connection's TLS handshake may take.
const HANDSHAKE: Duration = Duration::from_secs(10);
/// How long a request's headers may take to arrive, the first request's from the end of the handshake and each next
/// one's from the end of the last response: an idle connection is closed after that too.
const HEADERS: Duration = Duration::from_secs(10);
/// The longest request head, and about the most of a connection's input buffered (a read may fill what the buffer
/// reserved past it).
const MAX_BUFFER: usize = 64 * 1024;

/// Serves `router` on `listen` for as long as the admin runs.
pub async fn serve(listen: SocketAddr, tls: Arc<rustls::ServerConfig>, router: Router) -> io::Result<()> {
    let listener = TcpListener::bind(listen).await?;
    let tls = tokio_rustls::TlsAcceptor::from(tls);
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let http = http1(HEADERS);
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            // Out of file descriptors, say: one may be free once a connection closes.
            Err(e) => {
                tracing::warn!(%e, "accepting a connection failed");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            tracing::debug!("a connection was refused: too many open");
            continue;
        };
        // TCP_NODELAY: a video message smaller than the segment size must not wait for the previous one's ACK.
        let _ = stream.set_nodelay(true);
        let (tls, http, router) = (tls.clone(), http.clone(), router.clone());
        tokio::spawn(async move {
            // Beneath TLS, so that the handshake's writes, the responses' and an upgraded stream's all have the deadline.
            let stream = Deadline::new(stream, deadline::WRITE);
            if let Ok(Ok(stream)) = tokio::time::timeout(HANDSHAKE, tls.accept(stream)).await {
                let _ = connection(&http, stream, router).await;
            }
            drop(slot);
        });
    }
}

/// HTTP/1.1 as the server speaks it, a request's headers given `headers` to arrive.
fn http1(headers: Duration) -> http1::Builder {
    let mut http = http1::Builder::new();
    http.timer(TokioTimer::new()).header_read_timeout(headers).max_buf_size(MAX_BUFFER).max_header_size(MAX_BUFFER);
    http
}

/// One connection, served until it closes or is upgraded.
async fn connection<S>(http: &http1::Builder, stream: S, router: Router) -> hyper::Result<()>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    http.serve_connection(TokioIo::new(stream), TowerToHyperService::new(router)).with_upgrades().await
}

#[cfg(test)]
mod tests {
    use axum::extract::WebSocketUpgrade;
    use axum::extract::ws::Message;
    use axum::routing::get;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    use super::*;

    /// How long a request's headers may take in these tests, and the write deadline they serve with.
    const SHORT: Duration = Duration::from_millis(300);

    /// `router` served over plain TCP as `serve` serves it over TLS, with [`SHORT`] deadlines; and its address.
    async fn served(router: Router) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let http = http1(SHORT);
            while let Ok((stream, _)) = listener.accept().await {
                let (http, router) = (http.clone(), router.clone());
                tokio::spawn(async move { connection(&http, Deadline::new(stream, SHORT), router).await });
            }
        });
        address
    }

    /// Everything the server sends until it closes the connection, or what it sent within `within`.
    async fn received(stream: &mut TcpStream, within: Duration) -> (Vec<u8>, bool) {
        let mut all = Vec::new();
        let closed = tokio::time::timeout(within, stream.read_to_end(&mut all)).await.is_ok();
        (all, closed)
    }

    #[tokio::test]
    async fn headers_that_never_end_and_an_http2_preface_are_closed_and_a_request_is_answered() {
        let address = served(Router::new().route("/", get(|| async { "hello" }))).await;
        let mut slow = TcpStream::connect(address).await.unwrap();
        slow.write_all(b"GET / HTTP/1.1\r\nHost: a\r\nX-Slow: ").await.unwrap();
        let (_, closed) = received(&mut slow, SHORT * 10).await;
        assert!(closed, "closed at the header deadline");

        let mut h2 = TcpStream::connect(address).await.unwrap();
        h2.write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n\0\0\0\x04\0\0\0\0\0").await.unwrap();
        let (sent, closed) = received(&mut h2, SHORT * 10).await;
        // An HTTP/2 server would answer with its SETTINGS frame, type 4 at the fourth byte.
        assert!(closed && sent.get(3) != Some(&4), "{sent:?}");

        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(b"GET / HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n").await.unwrap();
        let (sent, _) = received(&mut client, SHORT * 10).await;
        let sent = String::from_utf8(sent).unwrap();
        assert!(sent.starts_with("HTTP/1.1 200") && sent.ends_with("hello"), "{sent}");
    }

    #[tokio::test]
    async fn headers_larger_than_the_buffer_are_refused() {
        let address = served(Router::new().route("/", get(|| async { "hello" }))).await;
        let mut client = TcpStream::connect(address).await.unwrap();
        let large = format!("GET / HTTP/1.1\r\nHost: a\r\nX-Large: {}\r\n\r\n", "a".repeat(MAX_BUFFER));
        let _ = client.write_all(large.as_bytes()).await;
        // A 431, or a reset that loses it: the request never reaches the route.
        let (sent, closed) = received(&mut client, SHORT * 10).await;
        let sent = String::from_utf8_lossy(&sent);
        assert!(closed && !sent.starts_with("HTTP/1.1 200"), "{sent}");
    }

    #[tokio::test]
    async fn a_client_that_stops_reading_is_closed_and_what_its_answer_held_goes_back() {
        let budget: &'static Semaphore = Box::leak(Box::new(Semaphore::new(64 * 1024)));
        let router = Router::new().route(
            "/",
            get(move || async move {
                let held = budget.try_acquire_many(32 * 1024).unwrap();
                // Far more than the sockets between them hold.
                let answer = serde_json::json!({"text": "a".repeat(32 << 20)});
                crate::service::respond(axum::http::StatusCode::OK, answer, Some(held))
            }),
        );
        let address = served(router).await;
        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(b"GET / HTTP/1.1\r\nHost: a\r\n\r\n").await.unwrap();
        let answered = async {
            while budget.available_permits() == 64 * 1024 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), answered).await.expect("answered");
        // Its bytes wait for the client, still counted, until the write deadline closes the connection.
        tokio::time::sleep(SHORT / 2).await;
        assert!(budget.available_permits() < 64 * 1024, "still held while the client may read");
        let given_back = async {
            while budget.available_permits() < 64 * 1024 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), given_back).await.expect("given back once the write failed");
        drop(client);
    }

    #[tokio::test]
    async fn an_idle_long_request_and_an_idle_websocket_are_never_cut() {
        let router = Router::new()
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(SHORT * 5).await;
                    "done"
                }),
            )
            .route(
                "/ws",
                get(|ws: WebSocketUpgrade| async {
                    ws.on_upgrade(|mut socket| async move {
                        while let Some(Ok(Message::Text(text))) = socket.recv().await {
                            let _ = socket.send(Message::Text(text)).await;
                        }
                    })
                }),
            );
        let address = served(router).await;
        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(b"GET /slow HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n").await.unwrap();
        let (sent, _) = received(&mut client, SHORT * 20).await;
        assert!(String::from_utf8(sent).unwrap().ends_with("done"), "answered after five header deadlines");

        let mut ws = TcpStream::connect(address).await.unwrap();
        let upgrade = "GET /ws HTTP/1.1\r\nHost: a\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
                       Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";
        ws.write_all(upgrade.as_bytes()).await.unwrap();
        let mut head = vec![0u8; 1024];
        let n = ws.read(&mut head).await.unwrap();
        assert!(head[..n].starts_with(b"HTTP/1.1 101"), "{}", String::from_utf8_lossy(&head[..n]));
        // Idle for longer than both deadlines, then one masked text frame, "hi", which comes back.
        tokio::time::sleep(SHORT * 5).await;
        let mask = [1u8, 2, 3, 4];
        ws.write_all(&[0x81, 0x82, 1, 2, 3, 4, b'h' ^ mask[0], b'i' ^ mask[1]]).await.unwrap();
        let mut echoed = [0u8; 4];
        tokio::time::timeout(SHORT * 10, ws.read_exact(&mut echoed)).await.expect("echoed").unwrap();
        assert_eq!(echoed, [0x81, 2, b'h', b'i']);
    }
}
