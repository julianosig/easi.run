//! The accept loop, the per-request decision, and the byte relay.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use egress::destination::{self, BenchFixture, Forbidden};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixListener};
use tokio::sync::Semaphore;
use tokio::time::{Instant, sleep_until, timeout};

use crate::log::{Record, Sink};
use crate::request::{self, Host, Request};
use crate::resolve::Resolver;
use crate::upstream::{Boxed, Dialer, Target};

/// Connections that may concurrently receive a 503 beyond the tunnel limit; the rest are closed without a reply.
const REFUSAL_SLOTS: usize = 64;
/// How long a refused client gets to read its response before the connection is dropped.
const REFUSAL_LINGER: Duration = Duration::from_secs(1);
const RELAY_BUFFER: usize = 16 * 1024;
/// The largest plain HTTP response head passed on, and its most header lines.
const MAX_RESPONSE_HEAD: usize = 64 * 1024;
const MAX_RESPONSE_HEADERS: usize = 256;

/// Runtime configuration.
pub struct Config {
    pub max_tunnels: usize,
    pub bench_fixture: Option<BenchFixture>,
    /// The public demo's walled garden: every destination, whatever its name or address, goes to the fixture, and
    /// nothing is ever resolved or dialled elsewhere. The fixture answers an unknown name by sending the page home.
    pub fixture_only: bool,
    /// The deployment's upstream proxy, when the session's traffic leaves through one: every connection goes through it
    /// (names are then resolved through it too; see `doh`), and none ever goes out directly.
    pub upstream: Option<Arc<Dialer>>,
    pub head_timeout: Duration,
    pub connect_timeout: Duration,
    pub idle_timeout: Duration,
    /// Test hook: after a destination is vetted, dial this local address instead. Absent from production builds.
    #[cfg(test)]
    pub test_upstreams: Vec<(SocketAddr, SocketAddr)>,
}

impl Config {
    pub fn new(max_tunnels: usize, bench_fixture: Option<BenchFixture>) -> Self {
        Self {
            max_tunnels,
            bench_fixture,
            fixture_only: false,
            upstream: None,
            head_timeout: Duration::from_secs(10),
            connect_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(300),
            #[cfg(test)]
            test_upstreams: Vec::new(),
        }
    }

    fn dial_target(&self, vetted: SocketAddr) -> SocketAddr {
        #[cfg(test)]
        if let Some((_, local)) = self.test_upstreams.iter().find(|(from, _)| *from == vetted) {
            return *local;
        }
        vetted
    }
}

struct Context {
    config: Config,
    resolver: Arc<dyn Resolver>,
    sink: Sink,
}

/// A refusal: the response status and the logged reason.
type Refusal = (u16, &'static str);

/// Serves TCP clients until the listener fails permanently.
pub async fn serve(listener: TcpListener, config: Config, resolver: Arc<dyn Resolver>, sink: Sink) {
    let admission = Admission::new(config, resolver, sink);
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(_) => {
                // Descriptor exhaustion and aborted handshakes are transient; back off instead of spinning.
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        admission.dispatch(stream, Some(peer));
    }
}

/// Serves clients on a Unix socket, accepting only `client_uid` (the browser, which has no network of its own).
pub async fn serve_unix(
    listener: UnixListener,
    client_uid: u32,
    config: Config,
    resolver: Arc<dyn Resolver>,
    sink: Sink,
) {
    let admission = Admission::new(config, resolver, sink);
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        // The peer's pid is 0 across containers; only its uid matters.
        if stream.peer_cred().map(|c| c.uid()).ok() != Some(client_uid) {
            (admission.ctx.sink)(Record { reason: "client_uid", ..Record::default() }.to_json());
            continue;
        }
        admission.dispatch(stream, None);
    }
}

/// Bounds concurrent requests and runs each on its own task.
struct Admission {
    tunnels: Arc<Semaphore>,
    refusals: Arc<Semaphore>,
    ctx: Arc<Context>,
}

impl Admission {
    fn new(config: Config, resolver: Arc<dyn Resolver>, sink: Sink) -> Admission {
        Admission {
            tunnels: Arc::new(Semaphore::new(config.max_tunnels)),
            refusals: Arc::new(Semaphore::new(REFUSAL_SLOTS)),
            ctx: Arc::new(Context { config, resolver, sink }),
        }
    }

    fn dispatch<S: Client>(&self, mut stream: S, peer: Option<SocketAddr>) {
        let ctx = self.ctx.clone();
        match self.tunnels.clone().try_acquire_owned() {
            Ok(permit) => {
                tokio::spawn(async move {
                    let _permit = permit;
                    handle(stream, peer, &ctx).await;
                });
            }
            Err(_) => {
                let record = Record { client: peer, reason: "tunnel_limit", ..Record::default() };
                (ctx.sink)(record.to_json());
                if let Ok(permit) = self.refusals.clone().try_acquire_owned() {
                    tokio::spawn(async move {
                        let _permit = permit;
                        refuse(&mut stream, 503).await;
                    });
                }
            }
        }
    }
}

/// A client connection: TCP, or a Unix socket.
trait Client: AsyncRead + AsyncWrite + Unpin + Send + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Client for T {}

async fn handle<S: Client>(mut client: S, peer: Option<SocketAddr>, ctx: &Context) {
    let start = Instant::now();
    let mut record = Record { client: peer, ..Record::default() };
    match proxy_one(&mut client, &mut record, ctx).await {
        Ok(reason) => {
            record.allowed = true;
            record.reason = reason;
        }
        Err((status, reason)) => {
            record.reason = reason;
            refuse(&mut client, status).await;
        }
    }
    record.duration_ms = start.elapsed().as_millis();
    (ctx.sink)(record.to_json());
}

async fn proxy_one<S: Client>(client: &mut S, record: &mut Record, ctx: &Context) -> Result<&'static str, Refusal> {
    let head = match request::read_head(client, ctx.config.head_timeout).await {
        Ok(head) => head,
        Err((error, method)) => {
            record.method = method;
            return Err(error.status());
        }
    };
    record.method = Some(head.method);
    let (host, port) = match &head.request {
        Request::Connect { host, port } | Request::Plain { host, port, .. } => (host, *port),
    };
    record.host = Some(host.display());
    record.port = Some(port);
    // Plain HTTP is forwarded on port 80 only; anything bound for 443 must tunnel.
    let port_ok = match &head.request {
        Request::Connect { .. } => destination::port_allowed(port),
        Request::Plain { .. } => port == 80,
    };
    if !port_ok {
        return Err((403, "port_not_allowed"));
    }
    let (addrs, fixture) = vetted_addresses(host, port, record, ctx).await?;
    let fixture = fixture.then(|| host.display());
    let mut upstream = connect(&addrs, port, fixture.as_deref(), record, ctx).await?;
    let mut sent = head.rest.len() as u64;
    match &head.request {
        Request::Connect { .. } => {
            client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .map_err(|_| (400, "client_closed"))?;
            upstream.write_all(&head.rest).await.map_err(|_| (502, "upstream_write_failed"))?;
        }
        Request::Plain { head: forwarded, .. } => {
            sent += forwarded.len() as u64;
            let mut bytes = Vec::with_capacity(forwarded.len() + head.rest.len());
            bytes.extend_from_slice(forwarded);
            bytes.extend_from_slice(&head.rest);
            upstream.write_all(&bytes).await.map_err(|_| (502, "upstream_write_failed"))?;
        }
    }
    record.bytes_up = sent;
    let plain = matches!(head.request, Request::Plain { .. });
    Ok(relay(client, &mut upstream, plain, ctx.config.idle_timeout, record).await)
}

/// Resolves the host once and vets every answer; one forbidden answer refuses the whole request. Also says whether the
/// host is the bench fixture.
async fn vetted_addresses(
    host: &Host,
    port: u16,
    record: &mut Record,
    ctx: &Context,
) -> Result<(Vec<IpAddr>, bool), Refusal> {
    let fixture = ctx.config.bench_fixture.as_ref();
    if ctx.config.fixture_only {
        // `main` refuses fixture_only without a fixture; a missing one here is still never a way out.
        let fixture = fixture.ok_or((403, "no_fixture"))?;
        return Ok((vec![IpAddr::V4(fixture.addr)], true));
    }
    let addrs = match host {
        Host::Ip(ip) => vec![*ip],
        Host::Name(name) => {
            if let Some(fixture) = fixture.filter(|f| f.name == *name) {
                return Ok((vec![IpAddr::V4(fixture.addr)], true));
            }
            match timeout(ctx.config.connect_timeout, ctx.resolver.resolve(name, port)).await {
                Err(_) => return Err((504, "dns_timeout")),
                Ok(Err(_)) => return Err((502, "dns_error")),
                Ok(Ok(addrs)) if addrs.is_empty() => return Err((502, "dns_no_answer")),
                Ok(Ok(addrs)) => addrs,
            }
        }
    };
    for addr in &addrs {
        if fixture.is_some_and(|f| destination::dial_address(*addr) == IpAddr::V4(f.addr)) {
            record.resolved = Some(*addr);
            return Err((403, "bench_fixture_address"));
        }
        if let Err(Forbidden(reason)) = destination::classify(*addr) {
            record.resolved = Some(*addr);
            return Err((403, reason));
        }
    }
    Ok((addrs, false))
}

/// Dials the vetted addresses in order within one connect timeout, through the upstream when there is one; nothing is
/// resolved again. The bench fixture goes to the upstream by name (the test upstream maps it), never by its address.
async fn connect(
    addrs: &[IpAddr],
    port: u16,
    fixture: Option<&str>,
    record: &mut Record,
    ctx: &Context,
) -> Result<Boxed, Refusal> {
    record.resolved = addrs.first().copied();
    if let Some(dialer) = &ctx.config.upstream {
        let attempt = async {
            if let Some(name) = fixture {
                return dialer.dial(Target::Name(name, port)).await.ok().map(|s| (s, addrs[0]));
            }
            for addr in addrs {
                let target = Target::Addr(SocketAddr::new(destination::dial_address(*addr), port));
                if let Ok(stream) = dialer.dial(target).await {
                    return Some((stream, *addr));
                }
            }
            None
        };
        return match timeout(ctx.config.connect_timeout, attempt).await {
            Err(_) => Err((504, "connect_timeout")),
            Ok(None) => Err((502, "upstream_failed")),
            Ok(Some((stream, addr))) => {
                record.resolved = Some(addr);
                Ok(stream)
            }
        };
    }
    let attempt = async {
        for addr in addrs {
            let target = ctx.config.dial_target(SocketAddr::new(destination::dial_address(*addr), port));
            if let Ok(stream) = TcpStream::connect(target).await {
                return Some((stream, *addr));
            }
        }
        None
    };
    match timeout(ctx.config.connect_timeout, attempt).await {
        Err(_) => Err((504, "connect_timeout")),
        Ok(None) => Err((502, "connect_failed")),
        Ok(Some((stream, addr))) => {
            let _ = stream.set_nodelay(true);
            record.resolved = Some(addr);
            Ok(Box::new(stream))
        }
    }
}

/// Relays both directions until both close, either fails, or neither moves a byte for `idle`. A direction's EOF is
/// passed on as a write shutdown, so half-closed streams keep flowing the other way. For a `plain` request, the
/// response's heads go first through [`response_heads`].
async fn relay<S: Client>(
    client: &mut S,
    upstream: &mut Boxed,
    plain: bool,
    idle: Duration,
    record: &mut Record,
) -> &'static str {
    let epoch = Instant::now();
    let last = AtomicU64::new(0);
    let (up, down) = (AtomicU64::new(0), AtomicU64::new(0));
    let (mut client_read, mut client_write) = tokio::io::split(client);
    let (mut upstream_read, mut upstream_write) = tokio::io::split(upstream);
    let both = async {
        tokio::try_join!(pipe(&mut client_read, &mut upstream_write, &up, &last, epoch), async {
            if plain {
                response_heads(&mut upstream_read, &mut client_write, &down, &last, epoch).await?;
            }
            pipe(&mut upstream_read, &mut client_write, &down, &last, epoch).await
        },)
    };
    let watchdog = async {
        loop {
            let deadline = epoch + Duration::from_millis(last.load(Ordering::Relaxed)) + idle;
            if Instant::now() >= deadline {
                return;
            }
            sleep_until(deadline).await;
        }
    };
    let reason = tokio::select! {
        result = both => if result.is_ok() { "closed" } else { "relay_error" },
        () = watchdog => "idle_timeout",
    };
    record.bytes_up += up.load(Ordering::Relaxed);
    record.bytes_down = down.load(Ordering::Relaxed);
    reason
}

async fn pipe<R, W>(
    reader: &mut R,
    writer: &mut W,
    count: &AtomicU64,
    last: &AtomicU64,
    epoch: Instant,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; RELAY_BUFFER];
    loop {
        let n = reader.read(&mut buf).await?;
        last.store(epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
        if n == 0 {
            return writer.shutdown().await;
        }
        writer.write_all(&buf[..n]).await?;
        count.fetch_add(n as u64, Ordering::Relaxed);
        last.store(epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
    }
}

/// A plain HTTP response head read so far.
#[derive(Debug, PartialEq)]
enum ResponseHead {
    Partial,
    /// An interim (1xx) head of this length, passed on unchanged.
    Interim(usize),
    /// The final head of this length, and what is sent in its place.
    Final(usize, Vec<u8>),
}

/// Parses the response head at the start of `bytes`. The final one is rebuilt to say `Connection: close`, with the
/// server's own `Connection`, `Proxy-Connection` and `Keep-Alive` dropped (a client honours the first it finds).
fn response_head(bytes: &[u8]) -> io::Result<ResponseHead> {
    let refused = || io::Error::from(io::ErrorKind::InvalidData);
    let mut headers = [httparse::EMPTY_HEADER; MAX_RESPONSE_HEADERS];
    let mut response = httparse::Response::new(&mut headers);
    let len = match response.parse(bytes) {
        Ok(httparse::Status::Complete(len)) if len <= MAX_RESPONSE_HEAD => len,
        Ok(httparse::Status::Partial) if bytes.len() <= MAX_RESPONSE_HEAD => return Ok(ResponseHead::Partial),
        _ => return Err(refused()),
    };
    let code = response.code.ok_or_else(refused)?;
    // 101 is never asked for (`Upgrade` is not forwarded), so it is final like any other.
    if (100..200).contains(&code) && code != 101 {
        return Ok(ResponseHead::Interim(len));
    }
    let mut out = Vec::with_capacity(len + 32);
    let version = response.version.ok_or_else(refused)?;
    out.extend_from_slice(format!("HTTP/1.{version} {code} {}\r\n", response.reason.unwrap_or_default()).as_bytes());
    for header in response.headers.iter() {
        if ["connection", "proxy-connection", "keep-alive"].iter().any(|h| header.name.eq_ignore_ascii_case(h)) {
            continue;
        }
        out.extend_from_slice(header.name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(header.value);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"Connection: close\r\n\r\n");
    Ok(ResponseHead::Final(len, out))
}

/// Passes on a plain HTTP response's heads, the final one saying `Connection: close`, then whatever followed it. One
/// request is relayed per plain connection: told to close, the client never sends a later request (perhaps for another
/// host, vetted for none) down this connection to this server, whatever the server says or however long it keeps the
/// connection open. A head that is malformed or too large ends the connection with nothing of it passed on.
async fn response_heads<R, W>(
    reader: &mut R,
    writer: &mut W,
    count: &AtomicU64,
    last: &AtomicU64,
    epoch: Instant,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = vec![0u8; RELAY_BUFFER];
    loop {
        let sent = match response_head(&buf)? {
            ResponseHead::Partial => {
                let n = reader.read(&mut chunk).await?;
                last.store(epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
                if n == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                buf.extend_from_slice(&chunk[..n]);
                continue;
            }
            ResponseHead::Interim(len) => {
                writer.write_all(&buf[..len]).await?;
                buf.drain(..len);
                len
            }
            ResponseHead::Final(len, head) => {
                writer.write_all(&head).await?;
                writer.write_all(&buf[len..]).await?;
                count.fetch_add((head.len() + buf.len() - len) as u64, Ordering::Relaxed);
                last.store(epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
                return Ok(());
            }
        };
        count.fetch_add(sent as u64, Ordering::Relaxed);
        last.store(epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
    }
}

/// Sends a bodiless error response, closes the write side, and briefly drains what the client already sent so
/// the close does not reset the connection before the client reads the response.
async fn refuse<S: Client>(stream: &mut S, status: u16) {
    let phrase = match status {
        400 => "Bad Request",
        403 => "Forbidden",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        431 => "Request Header Fields Too Large",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Gateway Timeout",
    };
    let response = format!("HTTP/1.1 {status} {phrase}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
    let _ = timeout(REFUSAL_LINGER, async {
        stream.write_all(response.as_bytes()).await?;
        stream.shutdown().await?;
        let mut sink = [0u8; 4096];
        let mut drained = 0usize;
        while drained < 64 * 1024 {
            match stream.read(&mut sink).await? {
                0 => break,
                n => drained += n,
            }
        }
        io::Result::Ok(())
    })
    .await;
}
