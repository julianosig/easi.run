//! End-to-end tests: the proxy on 127.0.0.1:0 with an injected resolver and test upstreams.
//!
//! Destinations are public-looking addresses; only after the proxy has vetted one does the `cfg(test)` hook dial a
//! local test upstream in its place. Production code has no loopback exception.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use egress::destination::BenchFixture;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::log::Sink;
use crate::resolve::{Resolution, Resolver};
use crate::server::{self, Config};

const PUBLIC: &str = "93.184.216.34";
const WAIT: Duration = Duration::from_secs(5);

struct MapResolver(HashMap<String, Vec<IpAddr>>);

impl Resolver for MapResolver {
    fn resolve<'a>(&'a self, name: &'a str, _port: u16) -> Resolution<'a> {
        let answer = self.0.get(name).cloned().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound));
        Box::pin(async move { answer })
    }
}

struct Proxy {
    addr: SocketAddr,
    logs: Arc<Mutex<Vec<String>>>,
}

impl Proxy {
    async fn start(config: Config, answers: &[(&str, &[&str])]) -> Self {
        let resolver = MapResolver(
            answers
                .iter()
                .map(|(name, addrs)| (name.to_string(), addrs.iter().map(|a| a.parse().unwrap()).collect()))
                .collect(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let logs = Arc::new(Mutex::new(Vec::new()));
        let sink: Sink = {
            let logs = logs.clone();
            Arc::new(move |line: String| logs.lock().unwrap().push(line))
        };
        tokio::spawn(server::serve(listener, config, Arc::new(resolver), sink));
        Self { addr, logs }
    }

    /// Sends `request` and returns everything the proxy answers before closing.
    async fn exchange(&self, request: &[u8]) -> String {
        let mut stream = TcpStream::connect(self.addr).await.unwrap();
        stream.write_all(request).await.unwrap();
        let mut out = Vec::new();
        timeout(WAIT, stream.read_to_end(&mut out)).await.expect("proxy did not close").unwrap();
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Opens a CONNECT tunnel and returns the stream after a 200 response head.
    async fn tunnel(&self, target: &str) -> TcpStream {
        let mut stream = TcpStream::connect(self.addr).await.unwrap();
        stream.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes()).await.unwrap();
        let head = read_head(&mut stream).await;
        assert!(head.starts_with("HTTP/1.1 200 "), "{target}: {head}");
        stream
    }

    /// Waits for a log line containing `needle` and returns it.
    async fn log_line(&self, needle: &str) -> String {
        timeout(WAIT, async {
            loop {
                if let Some(line) = self.logs.lock().unwrap().iter().find(|l| l.contains(needle)) {
                    return line.clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("no log line with {needle}: {:?}", self.logs.lock().unwrap()))
    }
}

async fn read_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        let n = timeout(WAIT, stream.read(&mut byte)).await.expect("no response head").unwrap();
        assert_eq!(n, 1, "closed after {:?}", String::from_utf8_lossy(&head));
        head.push(byte[0]);
    }
    String::from_utf8(head).unwrap()
}

fn status(response: &str) -> &str {
    response.split(' ').nth(1).unwrap_or("")
}

fn sock(text: &str) -> SocketAddr {
    text.parse().unwrap()
}

/// An upstream that reads until EOF, then answers `echo:` and the bytes, then closes.
async fn echo_upstream() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut data = Vec::new();
                stream.read_to_end(&mut data).await.unwrap();
                stream.write_all(b"echo:").await.unwrap();
                stream.write_all(&data).await.unwrap();
            });
        }
    });
    addr
}

/// An upstream that only counts connections: a refused destination must never reach it.
async fn alarm_upstream() -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        loop {
            let _ = listener.accept().await.unwrap();
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });
    (addr, hits)
}

fn config_with(upstreams: Vec<(SocketAddr, SocketAddr)>) -> Config {
    fixture_config(None, upstreams)
}

/// A configuration with the bench fixture `fixture` (`name=ipv4`), if any, and these test upstreams.
fn fixture_config(fixture: Option<&str>, upstreams: Vec<(SocketAddr, SocketAddr)>) -> Config {
    let mut config = Config::new(256, fixture.map(|f| BenchFixture::parse(f).unwrap()));
    config.test_upstreams = upstreams;
    config
}

/// Sends `bytes` down `stream`, closes its write side, and returns everything that comes back.
async fn reply(mut stream: TcpStream, bytes: &[u8]) -> Vec<u8> {
    stream.write_all(bytes).await.unwrap();
    stream.shutdown().await.unwrap();
    let mut reply = Vec::new();
    timeout(WAIT, stream.read_to_end(&mut reply)).await.unwrap().unwrap();
    reply
}

#[tokio::test]
async fn connect_tunnels_bytes_both_ways_with_half_close() {
    let echo = echo_upstream().await;
    let proxy = Proxy::start(config_with(vec![(sock("93.184.216.34:443"), echo)]), &[("public.test", &[PUBLIC])]).await;
    let mut stream = TcpStream::connect(proxy.addr).await.unwrap();
    // Bytes sent with the head must be relayed too.
    stream.write_all(b"CONNECT public.test:443 HTTP/1.1\r\nHost: public.test:443\r\n\r\nhel").await.unwrap();
    assert!(read_head(&mut stream).await.starts_with("HTTP/1.1 200 "));
    assert_eq!(reply(stream, b"lo").await, b"echo:hello");
    let line = proxy.log_line("\"result\":\"allowed\"").await;
    for field in [
        "\"method\":\"CONNECT\"",
        "\"host\":\"public.test\"",
        "\"port\":443",
        "\"resolved\":\"93.184.216.34\"",
        "\"reason\":\"closed\"",
        "\"bytes_up\":5",
        "\"bytes_down\":10",
        "\"client\":\"127.0.0.1:",
        "\"duration_ms\":",
        "\"ts\":\"",
    ] {
        assert!(line.contains(field), "{field} missing from {line}");
    }
}

#[tokio::test]
async fn a_destination_that_refuses_the_connection_gets_502() {
    // Nothing listens where the vetted address is dialed.
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
    let proxy = Proxy::start(config_with(vec![(sock("93.184.216.34:443"), dead)]), &[("public.test", &[PUBLIC])]).await;
    let response = proxy.exchange(b"CONNECT public.test:443 HTTP/1.1\r\nHost: public.test:443\r\n\r\n").await;
    assert_eq!(status(&response), "502", "{response}");
    proxy.log_line("\"reason\":\"connect_failed\"").await;
}

#[tokio::test]
async fn forbidden_destinations_are_refused_before_dialing() {
    let (alarm, hits) = alarm_upstream().await;
    let targets = [
        "127.0.0.1:443",
        "127.0.0.1:80",
        "10.0.0.1:443",
        "172.16.5.4:443",
        "192.168.1.10:443",
        "100.64.0.1:443",
        "169.254.169.254:80",
        "0.0.0.0:443",
        "255.255.255.255:443",
        "224.0.0.1:443",
        "[::1]:443",
        "[::]:443",
        "[::ffff:127.0.0.1]:443",
        "[::ffff:10.1.2.3]:443",
        "[64:ff9b::7f00:1]:443",
        "[64:ff9b::a9fe:a9fe]:443",
        "[64:ff9b:1::a00:1]:443",
        "[2002:7f00:1::]:443",
        "[fe80::1]:443",
        "[fd00::1]:443",
        "[ff02::1]:443",
        "private.test:443",
        "rebind.test:443",
        "metadata.test:80",
        "loop6.test:443",
        "mapped.test:443",
    ];
    let mut upstreams: Vec<(SocketAddr, SocketAddr)> = Vec::new();
    for ip in ["127.0.0.1", "10.0.0.1", "192.168.1.10", "169.254.169.254", "::1", "::ffff:127.0.0.1"] {
        let ip: IpAddr = ip.parse().unwrap();
        upstreams.push((SocketAddr::new(ip, 443), alarm));
        upstreams.push((SocketAddr::new(ip, 80), alarm));
    }
    let answers: &[(&str, &[&str])] = &[
        ("private.test", &["192.168.1.10"]),
        ("rebind.test", &["127.0.0.1"]),
        ("metadata.test", &["169.254.169.254"]),
        ("loop6.test", &["::1"]),
        ("mapped.test", &["::ffff:127.0.0.1"]),
    ];
    let proxy = Proxy::start(config_with(upstreams), answers).await;
    for target in targets {
        let response = proxy.exchange(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes()).await;
        assert_eq!(status(&response), "403", "CONNECT {target}: {response}");
    }
    for url in ["http://127.0.0.1/", "http://[::1]/", "http://metadata.test/latest/meta-data", "http://private.test/"] {
        let response = proxy.exchange(format!("GET {url} HTTP/1.1\r\n\r\n").as_bytes()).await;
        assert_eq!(status(&response), "403", "GET {url}: {response}");
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0, "a refused destination was dialed");
    let line = proxy.log_line("\"host\":\"rebind.test\"").await;
    assert!(line.contains("\"result\":\"refused\"") && line.contains("\"reason\":\"ipv4_loopback\""), "{line}");
    assert!(line.contains("\"resolved\":\"127.0.0.1\""), "{line}");
}

#[tokio::test]
async fn a_mixed_public_and_private_answer_is_refused() {
    let (alarm, hits) = alarm_upstream().await;
    let answers: &[(&str, &[&str])] = &[
        ("mixed.test", &[PUBLIC, "10.0.0.1"]),
        ("mixed-first.test", &["10.0.0.1", PUBLIC]),
        ("mixed-v6.test", &[PUBLIC, "::ffff:127.0.0.1"]),
        ("mixed-nat64.test", &[PUBLIC, "64:ff9b::a9fe:a9fe"]),
    ];
    let proxy = Proxy::start(config_with(vec![(sock("93.184.216.34:443"), alarm)]), answers).await;
    for name in ["mixed.test", "mixed-first.test", "mixed-v6.test", "mixed-nat64.test"] {
        let response = proxy.exchange(format!("CONNECT {name}:443 HTTP/1.1\r\n\r\n").as_bytes()).await;
        assert_eq!(status(&response), "403", "{name}: {response}");
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0, "the public answer was dialed");
}

#[tokio::test]
async fn ports_other_than_80_and_443_are_refused() {
    let (alarm, hits) = alarm_upstream().await;
    let upstreams = vec![(sock("93.184.216.34:22"), alarm), (sock("93.184.216.34:8080"), alarm)];
    let proxy = Proxy::start(config_with(upstreams), &[("public.test", &[PUBLIC])]).await;
    for request in [
        "CONNECT public.test:22 HTTP/1.1\r\n\r\n",
        "CONNECT 93.184.216.34:22 HTTP/1.1\r\n\r\n",
        "CONNECT public.test:8080 HTTP/1.1\r\n\r\n",
        "GET http://public.test:8080/ HTTP/1.1\r\n\r\n",
        "GET http://public.test:443/ HTTP/1.1\r\n\r\n",
    ] {
        let response = proxy.exchange(request.as_bytes()).await;
        assert_eq!(status(&response), "403", "{request:?}: {response}");
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    let line = proxy.log_line("\"port\":22").await;
    assert!(line.contains("\"reason\":\"port_not_allowed\""), "{line}");
}

#[tokio::test]
async fn oversized_and_malformed_heads_are_refused() {
    let proxy = Proxy::start(config_with(Vec::new()), &[("public.test", &[PUBLIC])]).await;
    let oversized = format!("GET http://public.test/ HTTP/1.1\r\nX-Pad: {}\r\n\r\n", "a".repeat(17 * 1024));
    assert_eq!(status(&proxy.exchange(oversized.as_bytes()).await), "431");
    let endless = "a".repeat(20 * 1024);
    assert_eq!(status(&proxy.exchange(endless.as_bytes()).await), "431");
    let many: String = (0..200).map(|i| format!("X-{i}: 1\r\n")).collect();
    let many = format!("GET http://public.test/ HTTP/1.1\r\n{many}\r\n");
    assert_eq!(status(&proxy.exchange(many.as_bytes()).await), "431");
    for (request, expected) in [
        ("GET /relative HTTP/1.1\r\nHost: public.test\r\n\r\n", "400"),
        ("GET https://public.test/ HTTP/1.1\r\n\r\n", "400"),
        ("GET http://user@public.test/ HTTP/1.1\r\n\r\n", "400"),
        ("CONNECT public.test HTTP/1.1\r\n\r\n", "400"),
        ("CONNECT public.test:443 HTTP/2.0\r\n\r\n", "400"),
        ("TRACE http://public.test/ HTTP/1.1\r\n\r\n", "405"),
        ("\x16\x03\x01\x02\x00\x01\x00\x01\x03\x03\r\n\r\n", "400"),
    ] {
        let response = proxy.exchange(request.as_bytes()).await;
        assert_eq!(status(&response), expected, "{request:?}: {response}");
    }
}

#[tokio::test]
async fn a_slow_head_times_out() {
    let mut config = config_with(Vec::new());
    config.head_timeout = Duration::from_millis(200);
    let proxy = Proxy::start(config, &[]).await;
    assert_eq!(status(&proxy.exchange(b"CONNECT public.test:443 HTTP/1.1\r\n").await), "408");
}

#[tokio::test]
async fn an_idle_tunnel_is_closed() {
    let echo = echo_upstream().await;
    let mut config = config_with(vec![(sock("93.184.216.34:443"), echo)]);
    config.idle_timeout = Duration::from_millis(200);
    let proxy = Proxy::start(config, &[("public.test", &[PUBLIC])]).await;
    let mut stream = proxy.tunnel("public.test:443").await;
    let mut rest = Vec::new();
    timeout(WAIT, stream.read_to_end(&mut rest)).await.expect("idle tunnel stayed open").unwrap();
    let line = proxy.log_line("\"reason\":\"idle_timeout\"").await;
    assert!(line.contains("\"result\":\"allowed\""), "{line}");
}

#[tokio::test]
async fn plain_http_is_forwarded_without_hop_by_hop_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = listener.local_addr().unwrap();
    let (heads, mut received) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let head = read_head(&mut stream).await;
        let mut body = [0u8; 4];
        stream.read_exact(&mut body).await.unwrap();
        heads.send((head, body)).unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").await.unwrap();
    });
    let proxy =
        Proxy::start(config_with(vec![(sock("93.184.216.34:80"), upstream)]), &[("public.test", &[PUBLIC])]).await;
    let request = "POST http://Public.Test/path/x?token=secret HTTP/1.1\r\n\
        Host: other.example\r\n\
        Connection: keep-alive, X-Hop\r\n\
        Keep-Alive: timeout=5\r\n\
        Proxy-Authorization: Basic Zm9vOmJhcg==\r\n\
        Proxy-Connection: keep-alive\r\n\
        TE: trailers\r\n\
        Trailer: X-T\r\n\
        Upgrade: h2c\r\n\
        X-Hop: dropped\r\n\
        X-Custom: kept\r\n\
        Content-Length: 4\r\n\
        \r\n\
        body";
    let response = proxy.exchange(request.as_bytes()).await;
    assert_eq!(response, "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    let (head, body) = timeout(WAIT, received.recv()).await.unwrap().unwrap();
    assert_eq!(
        head,
        "POST /path/x?token=secret HTTP/1.1\r\nHost: public.test\r\nX-Custom: kept\r\nContent-Length: 4\r\n\
         Connection: close\r\n\r\n"
    );
    assert_eq!(&body, b"body");
    let line = proxy.log_line("\"result\":\"allowed\"").await;
    assert!(line.contains("\"method\":\"POST\"") && line.contains("\"port\":80"), "{line}");
    for leaked in ["secret", "path", "token", "Basic", "kept"] {
        assert!(!line.contains(leaked), "{leaked} logged: {line}");
    }
}

/// A plain HTTP upstream that answers its first request with `answer` and then holds the connection open.
async fn holding_upstream(answer: &'static [u8]) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                read_head(&mut stream).await;
                stream.write_all(answer).await.unwrap();
                let mut rest = Vec::new();
                let _ = stream.read_to_end(&mut rest).await;
            });
        }
    });
    addr
}

#[tokio::test]
async fn a_plain_response_tells_the_client_to_close_whatever_the_server_says() {
    let upstream = holding_upstream(
        b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nConnection: keep-alive\r\nProxy-Connection: keep-alive\r\n\
          Keep-Alive: timeout=60\r\nContent-Length: 2\r\n\r\nok",
    )
    .await;
    let proxy =
        Proxy::start(config_with(vec![(sock("93.184.216.34:80"), upstream)]), &[("public.test", &[PUBLIC])]).await;
    let mut stream = TcpStream::connect(proxy.addr).await.unwrap();
    stream.write_all(b"GET http://public.test/ HTTP/1.1\r\nHost: public.test\r\n\r\n").await.unwrap();
    let expected = "HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
    let mut response = vec![0u8; expected.len()];
    timeout(WAIT, stream.read_exact(&mut response)).await.unwrap().unwrap();
    assert_eq!(String::from_utf8(response).unwrap(), expected);
    // A server that does not speak HTTP gets nothing through.
    let upstream = holding_upstream(b"not http\r\n\r\nHTTP/1.1 200 OK\r\n\r\n").await;
    let proxy =
        Proxy::start(config_with(vec![(sock("93.184.216.34:80"), upstream)]), &[("public.test", &[PUBLIC])]).await;
    let response = proxy.exchange(b"GET http://public.test/ HTTP/1.1\r\nHost: public.test\r\n\r\n").await;
    assert_eq!(response, "");
}

#[tokio::test]
async fn the_bench_fixture_admits_only_its_exact_name_and_address() {
    let echo = echo_upstream().await;
    let (alarm, hits) = alarm_upstream().await;
    let config = fixture_config(
        Some("Fixture.Bench=10.20.0.5"),
        vec![(sock("10.20.0.5:443"), echo), (sock("1.1.1.1:443"), alarm)],
    );
    let answers: &[(&str, &[&str])] = &[
        // DNS for the fixture name is never consulted.
        ("fixture.bench", &["1.1.1.1"]),
        ("alias.test", &["10.20.0.5"]),
        ("alias-mixed.test", &[PUBLIC, "10.20.0.5"]),
        ("alias-mapped.test", &["::ffff:10.20.0.5"]),
        ("sub.fixture.bench", &["10.20.0.5"]),
    ];
    let proxy = Proxy::start(config, answers).await;
    assert_eq!(reply(proxy.tunnel("fixture.bench:443").await, b"ping").await, b"echo:ping");
    proxy.tunnel("FIXTURE.bench.:443").await;
    for target in [
        "alias.test:443",
        "alias-mixed.test:443",
        "alias-mapped.test:443",
        "10.20.0.5:443",
        "[::ffff:10.20.0.5]:443",
        "fixture.bench:22",
        "10.20.0.6:443",
        "sub.fixture.bench:443",
    ] {
        let response = proxy.exchange(format!("CONNECT {target} HTTP/1.1\r\n\r\n").as_bytes()).await;
        assert_eq!(status(&response), "403", "{target}: {response}");
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0, "DNS was consulted for the fixture name");
    let line = proxy.log_line("\"host\":\"alias.test\"").await;
    assert!(line.contains("\"reason\":\"bench_fixture_address\""), "{line}");

    // A public fixture address is equally exclusive to its name.
    let (alarm, hits) = alarm_upstream().await;
    let config = fixture_config(Some("pub.bench=8.8.4.4"), vec![(sock("8.8.4.4:443"), alarm)]);
    let proxy = Proxy::start(config, &[("other.test", &["8.8.4.4"])]).await;
    let response = proxy.exchange(b"CONNECT other.test:443 HTTP/1.1\r\n\r\n").await;
    assert_eq!(status(&response), "403", "{response}");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn fixture_only_sends_every_destination_to_the_fixture_and_never_resolves() {
    let echo = echo_upstream().await;
    let (alarm, hits) = alarm_upstream().await;
    // Any real answer for these names would reach the alarm: the walled garden must never look them up or dial out.
    let mut config =
        fixture_config(Some("home.play=10.20.0.5"), vec![(sock("10.20.0.5:443"), echo), (sock("1.1.1.1:443"), alarm)]);
    config.fixture_only = true;
    let answers: &[(&str, &[&str])] = &[("example.com", &["1.1.1.1"]), ("upload.play", &["1.1.1.1"])];
    let proxy = Proxy::start(config, answers).await;
    for target in ["home.play:443", "upload.play:443", "example.com:443", "1.1.1.1:443", "[2606:4700::1]:443"] {
        assert_eq!(reply(proxy.tunnel(target).await, b"ping").await, b"echo:ping", "{target}");
    }
    let response = proxy.exchange(b"CONNECT example.com:22 HTTP/1.1\r\n\r\n").await;
    assert_eq!(status(&response), "403", "the port rule still holds: {response}");
    assert_eq!(hits.load(Ordering::SeqCst), 0, "a destination left the walled garden");
}

#[tokio::test]
async fn the_tunnel_limit_returns_503() {
    let echo = echo_upstream().await;
    let mut config = config_with(vec![(sock("93.184.216.34:443"), echo)]);
    config.max_tunnels = 2;
    let proxy = Proxy::start(config, &[("public.test", &[PUBLIC])]).await;
    let mut first = proxy.tunnel("public.test:443").await;
    let _second = proxy.tunnel("public.test:443").await;
    let mut excess = TcpStream::connect(proxy.addr).await.unwrap();
    let mut out = Vec::new();
    timeout(WAIT, excess.read_to_end(&mut out)).await.unwrap().unwrap();
    assert_eq!(status(&String::from_utf8_lossy(&out)), "503");
    proxy.log_line("\"reason\":\"tunnel_limit\"").await;

    // Closing a tunnel frees its slot.
    first.shutdown().await.unwrap();
    first.read_to_end(&mut Vec::new()).await.unwrap();
    drop(first);
    timeout(WAIT, async {
        loop {
            let mut stream = TcpStream::connect(proxy.addr).await.unwrap();
            stream.write_all(b"CONNECT public.test:443 HTTP/1.1\r\n\r\n").await.unwrap();
            let mut buf = [0u8; 12];
            if stream.read_exact(&mut buf).await.is_ok() && &buf == b"HTTP/1.1 200" {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the freed slot was not reused");
}

#[tokio::test]
async fn the_unix_listener_serves_only_its_client_uid() {
    let dir = std::env::temp_dir().join(format!("proxy-unix-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let me = rustix::process::getuid().as_raw();
    for (uid, served) in [(me, true), (me.wrapping_add(1), false)] {
        let path = dir.join(format!("p{uid}.sock"));
        let _ = std::fs::remove_file(&path);
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let resolver = MapResolver(Default::default());
        tokio::spawn(server::serve_unix(listener, uid, Config::new(4, None), Arc::new(resolver), Arc::new(|_| {})));
        let mut stream = tokio::net::UnixStream::connect(&path).await.unwrap();
        stream.write_all(b"CONNECT 127.0.0.1:443 HTTP/1.1\r\nHost: 127.0.0.1:443\r\n\r\n").await.unwrap();
        let mut response = Vec::new();
        let _ = timeout(WAIT, stream.read_to_end(&mut response)).await;
        let response = String::from_utf8_lossy(&response);
        if served {
            assert!(response.starts_with("HTTP/1.1 403"), "the policy answers its client: {response:?}");
        } else {
            assert!(response.is_empty(), "another uid gets no answer: {response:?}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// An HTTP upstream that answers each `CONNECT` with 200 and then echoes, keeping what it was asked for.
async fn connect_upstream() -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = asked.clone();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let seen = seen.clone();
            tokio::spawn(async move {
                let head = read_head(&mut stream).await;
                seen.lock().unwrap().push(head.lines().next().unwrap_or_default().to_owned());
                stream.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await.unwrap();
                let mut data = Vec::new();
                stream.read_to_end(&mut data).await.unwrap();
                stream.write_all(b"echo:").await.unwrap();
                stream.write_all(&data).await.unwrap();
            });
        }
    });
    (addr, asked)
}

fn config_through(upstream: SocketAddr, direct: Vec<(SocketAddr, SocketAddr)>, fixture: Option<&str>) -> Config {
    let url = format!("http://{upstream}");
    let dialer = crate::upstream::Dialer::new(egress::Upstream::parse_url(&url).unwrap()).unwrap();
    let mut config = fixture_config(fixture, direct);
    config.upstream = Some(Arc::new(dialer));
    config
}

#[tokio::test]
async fn with_an_upstream_every_connection_goes_through_it_by_the_vetted_address() {
    let (upstream, asked) = connect_upstream().await;
    // A direct path exists for the vetted address, and must not be taken.
    let (alarm, hits) = alarm_upstream().await;
    let config = config_through(upstream, vec![(sock("93.184.216.34:443"), alarm)], Some("fixture.bench=10.20.0.5"));
    let proxy = Proxy::start(config, &[("public.test", &[PUBLIC])]).await;
    assert_eq!(reply(proxy.tunnel("public.test:443").await, b"hi").await, b"echo:hi");
    // The bench fixture goes to the upstream by name.
    drop(proxy.tunnel("fixture.bench:443").await);
    proxy.log_line("\"host\":\"fixture.bench\"").await;
    assert_eq!(
        asked.lock().unwrap().clone(),
        ["CONNECT 93.184.216.34:443 HTTP/1.1", "CONNECT fixture.bench:443 HTTP/1.1"]
    );
    assert_eq!(hits.load(Ordering::SeqCst), 0, "a connection left directly");
}

#[tokio::test]
async fn with_an_upstream_that_fails_nothing_leaves_directly() {
    // Nothing listens at the upstream's address.
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
    let (alarm, hits) = alarm_upstream().await;
    let config = config_through(dead, vec![(sock("93.184.216.34:443"), alarm)], None);
    let proxy = Proxy::start(config, &[("public.test", &[PUBLIC]), ("private.test", &["10.0.0.7"])]).await;
    let response = proxy.exchange(b"CONNECT public.test:443 HTTP/1.1\r\nHost: public.test:443\r\n\r\n").await;
    assert_eq!(status(&response), "502", "{response}");
    proxy.log_line("\"reason\":\"upstream_failed\"").await;
    // Private answers are refused before the upstream is even asked.
    let response = proxy.exchange(b"CONNECT private.test:443 HTTP/1.1\r\nHost: private.test:443\r\n\r\n").await;
    assert_eq!(status(&response), "403", "{response}");
    assert_eq!(hits.load(Ordering::SeqCst), 0, "a connection left directly");
}
