//! The HTTPS server: each connection's TLS handshake, then the protocol it agreed on (see `tls`): HTTP/2 for a client
//! that asked for it, whose requests then share one connection instead of waiting for one each (a page's first load
//! from far away: some 450 ms less at 215 ms), else HTTP/1.1, WebSocket upgrades included (a browser opens its
//! WebSockets over HTTP/1.1: this server offers no WebSocket over HTTP/2). It bounds connections before any route or
//! login check sees them: how many may be open at once, and how many requests answered at once over them all; how long
//! the handshake, a request's headers and its body may take, and an idle connection may stay; how much of a request's
//! head is buffered; and how long a write may wait for a client that reads nothing (see `deadline`). None of these
//! cuts a connection that waits on the admin: a request waiting for a long agent call, or an idle viewer stream.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes, HttpBody};
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{BoxError, Router};
use hyper::body::{Frame, Incoming};
use hyper::server::conn::{http1, http2};
use hyper::service::Service;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};

use crate::deadline::{self, Deadline};

/// Connections in their handshake or served as HTTP at once; past them, a new one is closed as it comes. A browser
/// opens a handful per origin, an agent one per call in flight. A WebSocket upgrade leaves the count: a viewer's stream
/// is opened only for a live login.
const MAX_CONNECTIONS: usize = 256;
/// Requests answered at once over every connection, each until its answer's body ends: as many as HTTP/1.1's
/// connections could carry, so that HTTP/2's (many requests to a connection) admit no more; past them, a 503.
const MAX_REQUESTS: usize = MAX_CONNECTIONS;
/// Requests one HTTP/2 connection may have open at once: far more than a page asks for together.
const MAX_STREAMS: u32 = 64;
/// Streams an HTTP/2 client may open and reset before the server has answered them, and streams the server may reset
/// for a client's errors, before the connection is closed: what "rapid reset" floods are cut by.
const MAX_PENDING_RESETS: usize = 20;
const MAX_ERROR_RESETS: usize = 1024;
/// How often an HTTP/2 connection is pinged while it is quiet, and how long its answer may take: a peer that is gone
/// is closed even while a request of its own still waits on the admin.
const PING: Duration = Duration::from_secs(60);
const PING_ANSWER: Duration = Duration::from_secs(20);
/// The most of an answer's body handed to HTTP/2 at once, HTTP/2's own frame size: how much its client must make room
/// for within the write deadline (as a write on HTTP/1.1 must see some of it taken), so a client still reading at
/// some 550 bytes a second is never cut.
const PIECE: usize = 16 * 1024;
/// How long a connection's TLS handshake may take.
const HANDSHAKE: Duration = Duration::from_secs(10);
/// How long a request's headers may take to arrive, the first request's from the end of the handshake and each next
/// one's from the end of the last response: an idle connection is closed after that too. As long as Chrome keeps a used
/// connection idle, so that a page asking again after a pause does not pay a new connection's round trips (two more,
/// some half a second from far away).
const HEADERS: Duration = Duration::from_secs(300);
/// The longest request head, and about the most of a connection's input buffered (a read may fill what the buffer
/// reserved past it).
const MAX_BUFFER: usize = 64 * 1024;
/// How long a request's body may take to arrive in all, from the end of its headers: the largest an agent call may send
/// (1 MiB) at some 35 KB/s. Login and enrollment read theirs before any check, so without it a client that sends a byte
/// now and then would hold a connection's place for as long as it likes.
const BODY: Duration = Duration::from_secs(30);

/// Serves `router` on `listen` for as long as the admin runs.
pub async fn serve(listen: SocketAddr, tls: Arc<rustls::ServerConfig>, router: Router) -> io::Result<()> {
    let router = bounded(router, BODY, MAX_REQUESTS);
    let listener = TcpListener::bind(listen).await?;
    let tls = tokio_rustls::TlsAcceptor::from(tls);
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let http = Http::new(HEADERS, deadline::WRITE);
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
                let _ = secure(&http, stream, router).await;
            }
            drop(slot);
        });
    }
}

/// Both protocols as the server speaks them, a request's headers given `idle` to arrive (HTTP/1.1), as an HTTP/2
/// connection with no request open may stay; and how long an HTTP/2 client may leave a piece of an answer untaken, or
/// a connection asked to close take to, before it is closed (`deadline::WRITE`, HTTP/1.1's write deadline).
#[derive(Clone)]
struct Http {
    http1: http1::Builder,
    http2: http2::Builder<TokioExecutor>,
    idle: Duration,
    stall: Duration,
}

impl Http {
    fn new(idle: Duration, stall: Duration) -> Self {
        let mut http1 = http1::Builder::new();
        http1.timer(TokioTimer::new()).header_read_timeout(idle).max_buf_size(MAX_BUFFER).max_header_size(MAX_BUFFER);
        let mut http2 = http2::Builder::new(TokioExecutor::new());
        http2
            .timer(TokioTimer::new())
            .max_concurrent_streams(MAX_STREAMS)
            .max_header_list_size(MAX_BUFFER as u32)
            .max_pending_accept_reset_streams(MAX_PENDING_RESETS)
            .max_local_error_reset_streams(MAX_ERROR_RESETS)
            .keep_alive_interval(PING)
            .keep_alive_timeout(PING_ANSWER);
        Self { http1, http2, idle, stall }
    }
}

/// `router` with each request's body given `body` to arrive in all, and at most `requests` answered at once (each until
/// its answer's body ends).
fn bounded(router: Router, body: Duration, requests: usize) -> Router {
    let answering = Arc::new(Semaphore::new(requests));
    router
        .layer(axum::middleware::from_fn(move |request: Request, next: Next| {
            next.run(request.map(|inner| Body::new(Bounded { inner, deadline: Box::pin(tokio::time::sleep(body)) })))
        }))
        .layer(axum::middleware::from_fn(move |request: Request, next: Next| {
            let place = answering.clone().try_acquire_owned();
            async move {
                let Ok(place) = place else {
                    return (StatusCode::SERVICE_UNAVAILABLE, "the server is answering as many requests as it takes")
                        .into_response();
                };
                next.run(request).await.map(|inner| Body::new(Holding { inner, _held: place }))
            }
        }))
}

/// An answer's body that keeps `_held` until it ends or is dropped: a request's place, or its connection's count.
struct Holding<T> {
    inner: Body,
    _held: T,
}

impl<T: Send + Unpin + 'static> HttpBody for Holding<T> {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }
}

/// A request's body that fails once its deadline passes before it ended. The deadline is never moved by what arrives,
/// so a body that trickles is cut as one that stalls; a handler that never reads its body never waits for it.
struct Bounded {
    inner: Body,
    deadline: Pin<Box<tokio::time::Sleep>>,
}

impl HttpBody for Bounded {
    type Data = Bytes;
    type Error = BoxError;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BoxError>>> {
        if self.deadline.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Some(Err("the request's body took too long to arrive".into())));
        }
        Pin::new(&mut self.inner).poll_frame(cx).map_err(Into::into)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }
}

/// One TLS connection, in the protocol its handshake agreed on.
async fn secure<S>(http: &Http, stream: tokio_rustls::server::TlsStream<S>, router: Router) -> hyper::Result<()>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    if stream.get_ref().1.alpn_protocol() == Some(b"h2") {
        multiplexed(http, stream, router).await
    } else {
        connection(&http.http1, stream, router).await
    }
}

/// One HTTP/1.1 connection, served until it closes or is upgraded.
async fn connection<S>(http: &http1::Builder, stream: S, router: Router) -> hyper::Result<()>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    http.serve_connection(TokioIo::new(stream), TowerToHyperService::new(router)).with_upgrades().await
}

/// One HTTP/2 connection, served until it closes, or until it has had no request open for `http.idle` (as HTTP/1.1
/// closes a connection whose next request's headers do not come): a request still answering is never cut by it. Its
/// client must also take each piece of an answer within `http.stall`: HTTP/2 holds what the client has not made room
/// for without the socket's write ever waiting, so a client that never does would keep its requests' places forever.
async fn multiplexed<S>(http: &Http, stream: S, router: Router) -> hyper::Result<()>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let (open, mut changes) = watch::channel(0usize);
    let owed = Arc::new(Owed::default());
    let service = Counted { inner: TowerToHyperService::new(router), open: Arc::new(open), owed: owed.clone() };
    let connection = http.http2.serve_connection(TokioIo::new(stream), service);
    tokio::pin!(connection);
    let mut checks = tokio::time::interval(http.stall / 4);
    let mut closing = false;
    loop {
        let idle = *changes.borrow_and_update() == 0;
        tokio::select! {
            done = connection.as_mut() => return done,
            _ = changes.changed() => {}
            _ = checks.tick(), if !idle => {
                if owed.oldest().is_some_and(|since| since.elapsed() >= http.stall) {
                    return Ok(());
                }
            }
            () = tokio::time::sleep(if closing { http.stall } else { http.idle }), if idle => {
                // Asked to close, with no request open since: one that never finished its own handshake would wait
                // for it forever. A request it took meanwhile is answered first.
                if closing {
                    return Ok(());
                }
                connection.as_mut().graceful_shutdown();
                closing = true;
            }
        }
    }
}

/// A connection's service that counts its requests open, each until its answer's body ends, and follows its answers'
/// pieces (see `Owed`).
struct Counted<S> {
    inner: S,
    open: Arc<watch::Sender<usize>>,
    owed: Arc<Owed>,
}

/// One of a connection's requests open, counted until dropped.
struct Open(Arc<watch::Sender<usize>>);

impl Open {
    fn new(open: &Arc<watch::Sender<usize>>) -> Self {
        open.send_modify(|n| *n += 1);
        Self(open.clone())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        self.0.send_modify(|n| *n -= 1);
    }
}

/// An HTTP/2 connection's answers that wait on their client, each since when: an answer whose head was sent, or whose
/// last piece was handed over, until HTTP/2 asks for its next one (which it does once the client made room for that
/// one). An answer whose body waits on the server itself (the next piece of a download, say) waits on no client.
#[derive(Default)]
struct Owed {
    waiting: Mutex<HashMap<u64, Instant>>,
    next: AtomicU64,
}

impl Owed {
    fn owe(&self, answer: u64) {
        self.waiting.lock().unwrap_or_else(|e| e.into_inner()).insert(answer, Instant::now());
    }

    fn settle(&self, answer: u64) {
        self.waiting.lock().unwrap_or_else(|e| e.into_inner()).remove(&answer);
    }

    fn oldest(&self) -> Option<Instant> {
        self.waiting.lock().unwrap_or_else(|e| e.into_inner()).values().min().copied()
    }
}

/// An HTTP/2 answer's body: handed over in pieces of at most [`PIECE`], each followed in its connection's `Owed`;
/// its request stays counted open until it ends or is dropped.
struct Answer {
    inner: Body,
    /// What is left of the last data frame the body gave, not handed over yet.
    rest: Bytes,
    id: u64,
    owed: Arc<Owed>,
    _open: Open,
}

impl Answer {
    fn new(inner: Body, owed: Arc<Owed>, open: Open) -> Self {
        let id = owed.next.fetch_add(1, Ordering::Relaxed);
        owed.owe(id);
        Self { inner, rest: Bytes::new(), id, owed, _open: open }
    }

    fn piece(&mut self) -> Frame<Bytes> {
        let piece = self.rest.split_to(self.rest.len().min(PIECE));
        self.owed.owe(self.id);
        Frame::data(piece)
    }
}

impl Drop for Answer {
    fn drop(&mut self) {
        self.owed.settle(self.id);
    }
}

impl HttpBody for Answer {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let this = &mut *self;
        if !this.rest.is_empty() {
            return Poll::Ready(Some(Ok(this.piece())));
        }
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => match frame.into_data() {
                Ok(data) => {
                    this.rest = data;
                    Poll::Ready(Some(Ok(this.piece())))
                }
                Err(frame) => {
                    this.owed.owe(this.id);
                    Poll::Ready(Some(Ok(frame)))
                }
            },
            other => {
                this.owed.settle(this.id);
                other
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.rest.is_empty() && self.inner.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        let mut hint = self.inner.size_hint();
        let rest = self.rest.len() as u64;
        // The upper bound first: a lower one past it panics (an inner body read to its end has an upper bound of 0).
        if let Some(upper) = hint.upper() {
            hint.set_upper(upper + rest);
        }
        hint.set_lower(hint.lower() + rest);
        hint
    }
}

impl<S> Service<axum::http::Request<Incoming>> for Counted<S>
where
    S: Service<axum::http::Request<Incoming>, Response = Response, Error = std::convert::Infallible>,
    S::Future: Send + 'static,
{
    type Response = axum::http::Response<Answer>;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, request: axum::http::Request<Incoming>) -> Self::Future {
        let open = Open::new(&self.open);
        let owed = self.owed.clone();
        let answer = self.inner.call(request);
        Box::pin(async move { Ok(answer.await?.map(|inner| Answer::new(inner, owed, open))) })
    }
}

#[cfg(test)]
mod tests {
    use axum::extract::WebSocketUpgrade;
    use axum::extract::ws::Message;
    use axum::routing::get;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    use super::*;

    /// How long a request's headers may take in these tests, and the write deadline they serve with; a body may take
    /// three times that.
    const SHORT: Duration = Duration::from_millis(300);

    /// `router` served over plain TCP as `serve` serves it over TLS, with [`SHORT`] deadlines; and its address.
    async fn served(router: Router) -> SocketAddr {
        let router = bounded(router, SHORT * 3, MAX_REQUESTS);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let http = Http::new(SHORT, SHORT).http1;
            while let Ok((stream, _)) = listener.accept().await {
                let (http, router) = (http.clone(), router.clone());
                tokio::spawn(async move { connection(&http, Deadline::new(stream, SHORT), router).await });
            }
        });
        address
    }

    /// Everything the server sends until it closes the connection, or what it sent within `within`.
    async fn received(stream: &mut (impl AsyncRead + Unpin), within: Duration) -> (Vec<u8>, bool) {
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
    async fn a_body_that_trickles_is_cut_at_its_deadline_and_one_in_time_is_read() {
        let router = Router::new().route("/", axum::routing::post(|body: Bytes| async move { body.len().to_string() }));
        let address = served(router).await;
        let (mut read, mut write) = TcpStream::connect(address).await.unwrap().into_split();
        write.write_all(b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 1000\r\n\r\n").await.unwrap();
        // A byte well within each deadline, but never all of it.
        let trickle = tokio::spawn(async move {
            while write.write_all(b"a").await.is_ok() {
                tokio::time::sleep(SHORT / 4).await;
            }
        });
        let (sent, closed) = received(&mut read, SHORT * 10).await;
        let sent = String::from_utf8_lossy(&sent);
        assert!(closed && !sent.starts_with("HTTP/1.1 200"), "closed at the body's deadline: {sent}");
        trickle.abort();

        let mut client = TcpStream::connect(address).await.unwrap();
        let request = format!(
            "POST / HTTP/1.1\r\nHost: a\r\nConnection: close\r\nContent-Length: 1000\r\n\r\n{}",
            "a".repeat(1000)
        );
        client.write_all(request.as_bytes()).await.unwrap();
        let (sent, _) = received(&mut client, SHORT * 10).await;
        let sent = String::from_utf8(sent).unwrap();
        assert!(sent.starts_with("HTTP/1.1 200") && sent.ends_with("1000"), "{sent}");
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
        let budget: &'static crate::agent::Budget = Box::leak(Box::new(crate::agent::Budget::new(64 * 1024)));
        let router = Router::new().route(
            "/",
            get(move || async move {
                let held = budget.take(32 * 1024).unwrap();
                // Far more than the sockets between them hold.
                let answer = serde_json::json!({"text": "a".repeat(32 << 20)});
                crate::service::respond(axum::http::StatusCode::OK, answer, Some(held))
            }),
        );
        let address = served(router).await;
        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(b"GET / HTTP/1.1\r\nHost: a\r\n\r\n").await.unwrap();
        let until = |done: fn(usize) -> bool| {
            tokio::time::timeout(Duration::from_secs(10), async move {
                while !done(budget.free()) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
        };
        until(|free| free < 64 * 1024).await.expect("answered");
        // Its bytes wait for the client, still counted, until the write deadline closes the connection.
        tokio::time::sleep(SHORT / 2).await;
        assert!(budget.free() < 64 * 1024, "still held while the client may read");
        until(|free| free == 64 * 1024).await.expect("given back once the write failed");
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

    /// `router` served over TLS as `serve` serves it, with [`SHORT`] deadlines and `requests` answered at once; its
    /// address, and a client that trusts its certificate and asks for `alpn`.
    async fn served_tls(
        router: Router,
        requests: usize,
    ) -> (SocketAddr, impl Fn(&[&[u8]]) -> tokio_rustls::TlsConnector) {
        let dir = crate::private::test_dir("server-tls");
        let tls = tokio_rustls::TlsAcceptor::from(crate::tls::server_config(&dir, &["localhost".to_owned()]).unwrap());
        let router = bounded(router, SHORT * 3, requests);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let http = Http::new(SHORT, SHORT);
            while let Ok((stream, _)) = listener.accept().await {
                let (tls, http, router) = (tls.clone(), http.clone(), router.clone());
                tokio::spawn(async move {
                    let stream = tls.accept(Deadline::new(stream, SHORT)).await.unwrap();
                    secure(&http, stream, router).await
                });
            }
        });
        let pem = std::fs::read(dir.join("tls-cert.pem")).unwrap();
        let connector = move |alpn: &[&[u8]]| {
            let mut roots = rustls::RootCertStore::empty();
            use rustls::pki_types::pem::PemObject;
            for cert in rustls::pki_types::CertificateDer::pem_slice_iter(&pem) {
                roots.add(cert.unwrap()).unwrap();
            }
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let mut config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth();
            config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
            tokio_rustls::TlsConnector::from(Arc::new(config))
        };
        (address, connector)
    }

    /// An HTTP/2 client over a new TLS connection to `address`, and the connection's task (done once it closes).
    async fn http2_client(
        address: SocketAddr,
        connector: &impl Fn(&[&[u8]]) -> tokio_rustls::TlsConnector,
    ) -> (hyper::client::conn::http2::SendRequest<String>, tokio::task::JoinHandle<()>) {
        let tcp = TcpStream::connect(address).await.unwrap();
        let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
        let stream = connector(&[b"h2"]).connect(name, tcp).await.unwrap();
        assert_eq!(stream.get_ref().1.alpn_protocol(), Some(&b"h2"[..]), "HTTP/2 agreed on");
        let (send, connection) =
            hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(stream)).await.unwrap();
        (
            send,
            tokio::spawn(async move {
                let _ = connection.await;
            }),
        )
    }

    async fn asked(send: &mut hyper::client::conn::http2::SendRequest<String>, path: &str) -> (StatusCode, String) {
        use http_body_util::BodyExt;
        let request = axum::http::Request::get(format!("https://localhost{path}")).body(String::new()).unwrap();
        let answer = send.clone().send_request(request).await.unwrap();
        let status = answer.status();
        let body = answer.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    fn slow_router() -> Router {
        Router::new().route("/", get(|| async { "hello" })).route(
            "/slow",
            get(|| async {
                tokio::time::sleep(SHORT * 5).await;
                "done"
            }),
        )
    }

    #[tokio::test]
    async fn a_client_that_asks_for_http2_is_answered_over_it_and_one_that_does_not_over_http11() {
        let (address, connector) = served_tls(slow_router(), MAX_REQUESTS).await;
        let (mut send, _) = http2_client(address, &connector).await;
        assert_eq!(asked(&mut send, "/").await, (StatusCode::OK, "hello".to_owned()));

        let tcp = TcpStream::connect(address).await.unwrap();
        let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
        let mut stream = connector(&[b"http/1.1"]).connect(name, tcp).await.unwrap();
        assert_eq!(stream.get_ref().1.alpn_protocol(), Some(&b"http/1.1"[..]));
        stream.write_all(b"GET / HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n").await.unwrap();
        let (sent, _) = received(&mut stream, SHORT * 10).await;
        let sent = String::from_utf8(sent).unwrap();
        assert!(sent.starts_with("HTTP/1.1 200") && sent.ends_with("hello"), "{sent}");
    }

    #[tokio::test]
    async fn an_http2_connection_with_no_request_open_is_closed_and_one_answering_a_long_request_is_not() {
        let (address, connector) = served_tls(slow_router(), MAX_REQUESTS).await;
        let (mut send, connection) = http2_client(address, &connector).await;
        assert_eq!(
            asked(&mut send, "/slow").await,
            (StatusCode::OK, "done".to_owned()),
            "answered after five idle deadlines"
        );
        tokio::time::timeout(SHORT * 10, connection).await.expect("closed once idle").unwrap();
    }

    #[tokio::test]
    async fn requests_past_those_answered_at_once_are_refused_and_their_places_come_back() {
        let (address, connector) = served_tls(slow_router(), 2).await;
        let (mut send, _) = http2_client(address, &connector).await;
        let (mut a, mut b, mut c) = (send.clone(), send.clone(), send.clone());
        let (a, b, c) = tokio::join!(asked(&mut a, "/slow"), asked(&mut b, "/slow"), async {
            tokio::time::sleep(SHORT).await;
            asked(&mut c, "/slow").await
        });
        let mut statuses = [a.0, b.0, c.0];
        statuses.sort();
        assert_eq!(statuses, [StatusCode::OK, StatusCode::OK, StatusCode::SERVICE_UNAVAILABLE]);
        assert_eq!(asked(&mut send, "/").await, (StatusCode::OK, "hello".to_owned()), "both places given back");
    }

    #[tokio::test]
    async fn an_http2_client_that_never_makes_room_for_an_answer_is_closed_and_its_place_comes_back() {
        let router = slow_router().route("/large", get(|| async { "a".repeat(1 << 20) }));
        let (address, connector) = served_tls(router, 1).await;
        let tcp = TcpStream::connect(address).await.unwrap();
        let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
        let stream = connector(&[b"h2"]).connect(name, tcp).await.unwrap();
        // No room for any of an answer's data, ever.
        let (send, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
            .initial_stream_window_size(0)
            .handshake::<_, String>(TokioIo::new(stream))
            .await
            .unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        let request = axum::http::Request::get("https://localhost/large").body(String::new()).unwrap();
        let head = send.clone().send_request(request).await.unwrap();
        assert_eq!(head.status(), StatusCode::OK);
        tokio::time::timeout(SHORT * 10, connection).await.expect("closed at the write deadline").unwrap();
        drop(head);

        let (mut send, _) = http2_client(address, &connector).await;
        assert_eq!(asked(&mut send, "/").await, (StatusCode::OK, "hello".to_owned()), "its one place given back");
    }

    #[tokio::test]
    async fn an_http2_connection_whose_client_never_starts_is_closed() {
        let (address, connector) = served_tls(slow_router(), MAX_REQUESTS).await;
        let tcp = TcpStream::connect(address).await.unwrap();
        let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
        let mut stream = connector(&[b"h2"]).connect(name, tcp).await.unwrap();
        assert_eq!(stream.get_ref().1.alpn_protocol(), Some(&b"h2"[..]));
        // No connection preface: the TLS handshake and nothing after it.
        let (_, closed) = received(&mut stream, SHORT * 10).await;
        assert!(closed, "closed after the idle and write deadlines");
    }

    #[tokio::test]
    async fn a_large_http2_answer_reaches_a_client_that_reads_it() {
        let router = slow_router().route("/large", get(|| async { "a".repeat(8 << 20) }));
        let (address, connector) = served_tls(router, MAX_REQUESTS).await;
        let (mut send, _) = http2_client(address, &connector).await;
        let (status, body) = asked(&mut send, "/large").await;
        assert_eq!((status, body.len()), (StatusCode::OK, 8 << 20));
    }

    #[tokio::test]
    async fn an_http2_client_that_reads_slowly_but_steadily_is_never_cut() {
        use http_body_util::BodyExt;
        let router = slow_router().route("/large", get(|| async { "a".repeat(256 * 1024) }));
        let (address, connector) = served_tls(router, MAX_REQUESTS).await;
        let tcp = TcpStream::connect(address).await.unwrap();
        let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
        let stream = connector(&[b"h2"]).connect(name, tcp).await.unwrap();
        // Room for one piece at a time, made a third of the write deadline after the last.
        let (send, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
            .initial_stream_window_size(PIECE as u32)
            .adaptive_window(false)
            .handshake::<_, String>(TokioIo::new(stream))
            .await
            .unwrap();
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let request = axum::http::Request::get("https://localhost/large").body(String::new()).unwrap();
        let mut body = send.clone().send_request(request).await.unwrap().into_body();
        let started = Instant::now();
        let mut read = 0;
        while let Some(frame) = body.frame().await {
            read += frame.unwrap().into_data().map_or(0, |data| data.len());
            tokio::time::sleep(SHORT / 3).await;
        }
        assert_eq!(read, 256 * 1024, "all of it, though it took {:?}", started.elapsed());
        assert!(started.elapsed() > SHORT * 3, "longer than the write deadline in all");
    }
}
