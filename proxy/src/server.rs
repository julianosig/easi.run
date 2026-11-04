//! The accept loop, the per-request decision, and the byte relay.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use egress::destination::{self, BenchFixture};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixListener};
use tokio::sync::Semaphore;
use tokio::time::{Instant, sleep_until, timeout};

use crate::log::{Record, Sink};
use crate::request::{self, Host};
use crate::resolve::Resolver;
use crate::stagger;
use crate::upstream::{Boxed, Dialer, Target};

/// Connections that may concurrently receive a 503 beyond the tunnel limit; the rest are closed without a reply.
const REFUSAL_SLOTS: usize = 64;
/// How long a refused client gets to read its response before the connection is dropped.
const REFUSAL_LINGER: Duration = Duration::from_secs(1);
const RELAY_BUFFER: usize = 16 * 1024;

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

/// The next connection `accept` takes. Descriptor exhaustion and aborted handshakes are transient: it backs off instead
/// of spinning.
async fn accepted<T, F: Future<Output = io::Result<T>>>(mut accept: impl FnMut() -> F) -> T {
    loop {
        match accept().await {
            Ok(accepted) => return accepted,
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

/// Serves TCP clients until the listener fails permanently.
pub async fn serve(listener: TcpListener, config: Config, resolver: Arc<dyn Resolver>, sink: Sink) {
    let admission = Admission::new(config, resolver, sink);
    loop {
        let (stream, peer) = accepted(|| listener.accept()).await;
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
        let (stream, _) = accepted(|| listener.accept()).await;
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
    record.method = Some("CONNECT".to_owned());
    let (host, port) = (&head.host, head.port);
    record.host = Some(host.display());
    record.port = Some(port);
    if !destination::port_allowed(port) {
        return Err((403, "port_not_allowed"));
    }
    let (addrs, fixture) = vetted_addresses(host, port, record, ctx).await?;
    let fixture = fixture.then(|| host.display());
    let mut upstream = connect(&addrs, port, fixture.as_deref(), record, ctx).await?;
    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.map_err(|_| (400, "client_closed"))?;
    // The tunnel is established: a failure now ends the connection, since any response written into it would reach the
    // client as tunnelled bytes.
    if upstream.write_all(&head.rest).await.is_err() {
        return Ok("upstream_write_failed");
    }
    record.bytes_up = head.rest.len() as u64;
    Ok(relay(client, &mut upstream, ctx.config.idle_timeout, record).await)
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
    if let Err((addr, reason)) = destination::vet(&addrs, fixture) {
        record.resolved = Some(addr);
        return Err((403, reason));
    }
    Ok((addrs, false))
}

/// Dials the vetted addresses, staggered (see `stagger`), within one connect timeout, through the upstream when there is
/// one; nothing is resolved again. The bench fixture goes to the upstream by name (the test upstream maps it), never by
/// its address.
async fn connect(
    addrs: &[IpAddr],
    port: u16,
    fixture: Option<&str>,
    record: &mut Record,
    ctx: &Context,
) -> Result<Boxed, Refusal> {
    record.resolved = addrs.first().copied();
    let upstream = ctx.config.upstream.as_ref();
    let attempt = async {
        if let (Some(dialer), Some(name)) = (upstream, fixture) {
            return dialer.dial(Target::Name(name, port)).await.ok().map(|s| (s, addrs[0]));
        }
        let dial = |addr| async move {
            let to = SocketAddr::new(destination::dial_address(addr), port);
            match upstream {
                Some(dialer) => dialer.dial(Target::Addr(to)).await,
                None => TcpStream::connect(ctx.config.dial_target(to)).await.map(|stream| {
                    let _ = stream.set_nodelay(true);
                    Box::new(stream) as Boxed
                }),
            }
        };
        stagger::first(addrs, dial).await.ok()
    };
    match timeout(ctx.config.connect_timeout, attempt).await {
        Err(_) => Err((504, "connect_timeout")),
        Ok(None) => Err((502, if upstream.is_some() { "upstream_failed" } else { "connect_failed" })),
        Ok(Some((stream, addr))) => {
            record.resolved = Some(addr);
            Ok(stream)
        }
    }
}

/// Relays both directions until both close, either fails, or neither moves a byte for `idle`. A direction's EOF is
/// passed on as a write shutdown, so half-closed streams keep flowing the other way.
async fn relay<S: Client>(client: &mut S, upstream: &mut Boxed, idle: Duration, record: &mut Record) -> &'static str {
    let epoch = Instant::now();
    let last = AtomicU64::new(0);
    let (up, down) = (AtomicU64::new(0), AtomicU64::new(0));
    let (mut client_read, mut client_write) = tokio::io::split(client);
    let (mut upstream_read, mut upstream_write) = tokio::io::split(upstream);
    let both = async {
        tokio::try_join!(
            pipe(&mut client_read, &mut upstream_write, &up, &last, epoch),
            pipe(&mut upstream_read, &mut client_write, &down, &last, epoch),
        )
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
        touch(last, epoch);
        if n == 0 {
            return writer.shutdown().await;
        }
        writer.write_all(&buf[..n]).await?;
        count.fetch_add(n as u64, Ordering::Relaxed);
        touch(last, epoch);
    }
}

/// Records that the relay moved now, in milliseconds since `epoch`.
fn touch(last: &AtomicU64, epoch: Instant) {
    last.store(epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
}

/// Sends a bodiless error response, closes the write side, and briefly drains what the client already sent so
/// the close does not reset the connection before the client reads the response.
async fn refuse<S: Client>(stream: &mut S, status: u16) {
    let phrase = hyper::StatusCode::from_u16(status).ok().and_then(|s| s.canonical_reason()).unwrap_or_default();
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
