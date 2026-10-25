//! The check of an upstream before sessions leave through it (`PROXY_CHECK=1`), run by the controller in a container of
//! its own on a network like a session proxy's, so the upstream is judged from where a session will reach it. It reaches
//! the upstream as a session's proxy does (the system resolver for its name, TLS verified against the public roots for
//! an `https` one) and goes through it as a session's first page does: a tunnel to the DNS-over-HTTPS resolver, by
//! address, with its TLS verified as `doh` verifies it; the trace page's name resolved through the upstream by that
//! resolver (`doh` itself); and, through a tunnel to the vetted address it gave, Cloudflare's public trace page fetched
//! over verified TLS (`GET https://www.cloudflare.com/cdn-cgi/trace`), which says the address the request came from and
//! its country. All of it within [`DEADLINE`].
//!
//! The outcome is one JSON line: `{"ok":true,"latency_ms":N,"ip":IP,"country":CC}`, or
//! `{"ok":false,"reason":R,"detail":D}`, where the reason is one of [`Reason`] and the detail a fixed sentence (at most
//! with a status number), never anything the upstream or the page sent, or the check was given.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egress::Scheme;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::resolve::Resolver;
use crate::upstream::{Answer, Boxed, Dialer, Target, tls, tls_config};

/// How long the whole check may take: three tunnels through the upstream, one DNS query among them.
pub const DEADLINE: Duration = Duration::from_secs(10);
/// The trace page's host and path.
const TRACE_HOST: &str = "www.cloudflare.com";
const TRACE_PATH: &str = "/cdn-cgi/trace";
/// The most of the trace page's answer read.
const MAX_TRACE: usize = 16 * 1024;

/// Why an upstream failed its check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Its name does not resolve, or nothing answers at its address and port.
    Unreachable,
    /// A step did not finish within the deadline.
    Timeout,
    /// It asks for credentials, or refused those given.
    Auth,
    /// It refused a tunnel, closed one before the far end answered, or names did not resolve through it.
    Refused,
    /// The TLS to an `https` upstream, or to the far end through a tunnel, did not verify or failed.
    Tls,
    /// It does not answer as the proxy its scheme names.
    Protocol,
    /// The trace page answered something else than its trace.
    Unexpected,
}

impl Reason {
    pub fn name(self) -> &'static str {
        match self {
            Reason::Unreachable => "unreachable",
            Reason::Timeout => "timeout",
            Reason::Auth => "auth",
            Reason::Refused => "refused",
            Reason::Tls => "tls",
            Reason::Protocol => "protocol",
            Reason::Unexpected => "unexpected",
        }
    }
}

/// How a check went.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The trace page answered: the address it saw the request come from, and that address's country.
    Passed {
        latency_ms: u64,
        ip: IpAddr,
        country: String,
    },
    Failed {
        reason: Reason,
        detail: String,
    },
}

impl Outcome {
    /// The one line the check prints.
    pub fn to_json(&self) -> String {
        match self {
            Outcome::Passed { latency_ms, ip, country } => {
                json!({"ok": true, "latency_ms": latency_ms, "ip": ip.to_string(), "country": country})
            }
            Outcome::Failed { reason, detail } => json!({"ok": false, "reason": reason.name(), "detail": detail}),
        }
        .to_string()
    }
}

fn failed(reason: Reason, detail: impl Into<String>) -> Outcome {
    Outcome::Failed { reason, detail: detail.into() }
}

/// What the check goes through the upstream to: in production, the resolver `doh` uses and Cloudflare's trace page.
pub struct Probe {
    /// The DNS-over-HTTPS resolver's address, the name its certificate is verified for, and its TLS.
    pub resolver: (SocketAddr, &'static str, Arc<rustls::ClientConfig>),
    /// What resolves the trace page's name: `doh` through the upstream.
    pub names: Arc<dyn Resolver>,
    /// The trace page's host, whose certificate is verified, and its TLS.
    pub host: &'static str,
    pub tls: Arc<rustls::ClientConfig>,
    pub deadline: Duration,
}

impl Probe {
    /// The check every upstream gets: through `dialer`'s upstream to the resolver every session's names go to, and to
    /// Cloudflare's trace page by the address that resolver gives.
    pub fn new(dialer: Arc<Dialer>) -> io::Result<Probe> {
        Ok(Probe {
            resolver: (crate::doh::SERVER, crate::doh::SERVER_NAME, tls_config(&[b"h2"])?),
            names: Arc::new(crate::doh::Doh::new(dialer)?),
            host: TRACE_HOST,
            tls: tls_config(&[b"http/1.1"])?,
            deadline: DEADLINE,
        })
    }
}

/// The step under way, which a timeout names.
#[derive(Clone, Copy)]
enum Step {
    Resolve,
    Connect,
    Handshake,
    Tunnel,
    Names,
    Site,
}

/// Checks the upstream `dialer` reaches, as `probe` says.
pub async fn check(dialer: &Dialer, probe: &Probe) -> Outcome {
    let started = Instant::now();
    let mut step = Step::Resolve;
    let seconds = probe.deadline.as_secs();
    match tokio::time::timeout(probe.deadline, steps(dialer, probe, &mut step)).await {
        Ok(Ok((ip, country))) => Outcome::Passed { latency_ms: started.elapsed().as_millis() as u64, ip, country },
        Ok(Err(outcome)) => outcome,
        Err(_) => failed(
            Reason::Timeout,
            match step {
                Step::Resolve => format!("the proxy's name did not resolve within {seconds} s"),
                Step::Connect => format!("the proxy did not answer within {seconds} s"),
                Step::Handshake => format!("the TLS handshake with the proxy did not finish within {seconds} s"),
                Step::Tunnel => format!("the proxy did not open a tunnel within {seconds} s"),
                Step::Names => format!("names did not resolve through the proxy within {seconds} s"),
                Step::Site => format!("the trace page did not answer through the proxy within {seconds} s"),
            },
        ),
    }
}

async fn steps(dialer: &Dialer, probe: &Probe, step: &mut Step) -> Result<(IpAddr, String), Outcome> {
    let upstream = dialer.upstream();
    let (host, port) = (upstream.host.as_str(), upstream.port);
    let unresolved = || failed(Reason::Unreachable, "the proxy's name does not resolve");
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port)).await.map_err(|_| unresolved())?.collect();
    if addrs.is_empty() {
        return Err(unresolved());
    }
    *step = Step::Connect;
    // Each address in turn, as the session's proxy connects to the upstream (`TcpStream::connect` of its name).
    let mut refused = false;
    let mut tcp = None;
    for addr in addrs {
        match TcpStream::connect(addr).await {
            Ok(stream) => {
                tcp = Some(stream);
                break;
            }
            Err(e) => refused |= e.kind() == io::ErrorKind::ConnectionRefused,
        }
    }
    let Some(tcp) = tcp else {
        let detail = if refused {
            "nothing listens at the proxy's address and port"
        } else {
            "the proxy's address cannot be reached"
        };
        return Err(failed(Reason::Unreachable, detail));
    };
    let _ = tcp.set_nodelay(true);
    let stream: Boxed = match dialer.tls() {
        Some(config) => {
            *step = Step::Handshake;
            tls(Box::new(tcp), host, config).await.map_err(|e| match certificate(&e) {
                Some(true) => failed(Reason::Tls, "the proxy's certificate did not verify for its name"),
                _ => failed(Reason::Tls, "the TLS handshake with the proxy failed"),
            })?
        }
        None => Box::new(tcp),
    };
    // The resolver's tunnel first, on this connection: what fails there fails every session's names.
    *step = Step::Tunnel;
    let (resolver, resolver_name, resolver_tls) = &probe.resolver;
    let tunnel = dialer.through(stream, Target::Addr(*resolver)).await.map_err(|e| tunnel_failure(dialer, &e))?;
    *step = Step::Site;
    tls(tunnel, resolver_name, resolver_tls.clone()).await.map_err(|e| far_failure(&e))?;
    *step = Step::Names;
    let names = probe.names.resolve(probe.host, 443).await.map_err(|e| names_failure(dialer, &e))?;
    let addr = names.into_iter().find(|a| egress::destination::classify(*a).is_ok());
    let addr = addr.ok_or_else(|| failed(Reason::Refused, "names did not resolve through the proxy"))?;
    *step = Step::Tunnel;
    let to = SocketAddr::new(egress::destination::dial_address(addr), 443);
    let tunnel = dialer.dial(Target::Addr(to)).await.map_err(|e| tunnel_failure(dialer, &e))?;
    *step = Step::Site;
    let mut page = tls(tunnel, probe.host, probe.tls.clone()).await.map_err(|e| far_failure(&e))?;
    let request = format!(
        // HTTP/1.0: the answer is the body itself up to the close, never chunked.
        "GET {TRACE_PATH} HTTP/1.0\r\nHost: {}\r\nAccept: text/plain\r\nUser-Agent: easi.run\r\nConnection: close\r\n\r\n",
        probe.host
    );
    let closed = || failed(Reason::Refused, "the proxy closed the tunnel before the trace page answered");
    page.write_all(request.as_bytes()).await.map_err(|_| closed())?;
    page.flush().await.map_err(|_| closed())?;
    let mut answer = Vec::new();
    let mut buf = [0u8; 4096];
    // To its end (a close without TLS's own close message included), or the most read.
    while answer.len() < MAX_TRACE {
        match page.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => answer.extend_from_slice(&buf[..n]),
        }
    }
    if answer.is_empty() {
        return Err(closed());
    }
    trace(&answer).ok_or_else(|| failed(Reason::Unexpected, "the trace page's answer is not its trace"))
}

/// The address and country a trace page's answer names: a 200 to the HTTP/1.0 request, not chunked, whose body holds
/// `ip=` and `loc=` lines.
fn trace(answer: &[u8]) -> Option<(IpAddr, String)> {
    let text = std::str::from_utf8(answer).ok()?;
    let (head, body) = text.split_once("\r\n\r\n")?;
    let mut lines = head.split("\r\n");
    let status = lines.next()?;
    if !(status.starts_with("HTTP/1.1 200 ") || status.starts_with("HTTP/1.0 200 ") || status.ends_with(" 200")) {
        return None;
    }
    // A framing other than the close's would have to be decoded first; an answer to HTTP/1.0 has none.
    if lines.any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:")) {
        return None;
    }
    let value = |key: &str| body.lines().find_map(|l| l.trim_end_matches('\r').strip_prefix(key));
    let ip: IpAddr = value("ip=")?.parse().ok()?;
    let country = value("loc=")?;
    let valid = country.len() == 2 && country.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
    valid.then(|| (ip, country.to_owned()))
}

/// Whether a TLS failure is TLS's own (`Some`), and then whether it is the certificate's; `None` for the connection's.
fn certificate(e: &io::Error) -> Option<bool> {
    let tls = e.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>())?;
    Some(matches!(tls, rustls::Error::InvalidCertificate(_)))
}

/// A TLS handshake through a tunnel that failed.
fn far_failure(e: &io::Error) -> Outcome {
    match certificate(e) {
        // The far end is not who it should be: possibly an upstream that intercepts TLS.
        Some(true) => failed(Reason::Tls, "a certificate did not verify through the proxy"),
        Some(false) => failed(Reason::Tls, "a TLS handshake through the proxy failed"),
        None => failed(Reason::Refused, "the proxy closed a tunnel before the far end answered"),
    }
}

/// A lookup through the upstream's resolver that failed: the tunnel's refusal, the resolver's TLS, its deadline, or
/// its answers.
fn names_failure(dialer: &Dialer, e: &io::Error) -> Outcome {
    if e.get_ref().is_some_and(|inner| inner.is::<Answer>()) {
        return tunnel_failure(dialer, e);
    }
    if certificate(e).is_some() {
        return far_failure(e);
    }
    if e.kind() == io::ErrorKind::TimedOut {
        return failed(Reason::Timeout, "names did not resolve through the proxy in time");
    }
    failed(Reason::Refused, "names did not resolve through the proxy")
}

fn tunnel_failure(dialer: &Dialer, e: &io::Error) -> Outcome {
    let upstream = dialer.upstream();
    let answer = e.get_ref().and_then(|inner| inner.downcast_ref::<Answer>());
    match answer {
        Some(Answer::Auth) if upstream.username.is_some() => {
            failed(Reason::Auth, "the proxy refused the user name or password")
        }
        Some(Answer::Auth) => failed(Reason::Auth, "the proxy asks for a user name and password"),
        Some(Answer::Status(code)) => failed(Reason::Refused, format!("the proxy refused the tunnel (HTTP {code})")),
        Some(Answer::Reply(code)) => {
            failed(Reason::Refused, format!("the proxy refused the tunnel (SOCKS5 reply {code})"))
        }
        Some(Answer::Malformed) => failed(
            Reason::Protocol,
            match upstream.scheme {
                Scheme::Http | Scheme::Https => "the proxy does not answer as an HTTP proxy",
                Scheme::Socks5 => "the proxy does not answer as a SOCKS5 proxy",
            },
        ),
        None if e.kind() == io::ErrorKind::ConnectionRefused => {
            failed(Reason::Unreachable, "nothing listens at the proxy's address and port")
        }
        None => failed(Reason::Protocol, "the proxy closed the connection before answering"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::Resolution;
    use egress::Upstream;
    use rustls::pki_types::CertificateDer;
    use tokio::net::TcpListener;

    /// The resolver's and the trace page's names in these tests, which one certificate covers, and their addresses.
    const RESOLVER: &str = "resolver.test";
    const SITE: &str = "site.test";
    const RESOLVER_AT: &str = "198.51.100.53:443";
    const SITE_IP: &str = "93.184.216.34";
    const TRACE: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nfl=1\nh=www.cloudflare.com\nip=203.0.113.9\nts=1\nloc=BR\ntls=TLSv1.3\n";

    fn certified(names: &[&str]) -> (CertificateDer<'static>, Arc<rustls::ServerConfig>) {
        let certified =
            rcgen::generate_simple_self_signed(names.iter().map(|n| n.to_string()).collect::<Vec<_>>()).unwrap();
        let cert = certified.cert.der().clone();
        let key = rustls::pki_types::PrivateKeyDer::try_from(certified.signing_key.serialize_der()).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.clone()], key)
            .unwrap();
        (cert, Arc::new(config))
    }

    fn trusting(cert: Option<CertificateDer<'static>>) -> Arc<rustls::ClientConfig> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add_parsable_certificates(cert);
        Arc::new(
            rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    }

    /// Names as the upstream's resolver would answer them: the site's, or none; or a lookup that timed out.
    struct Names(Option<bool>);

    impl Resolver for Names {
        fn resolve<'a>(&'a self, _name: &'a str, _port: u16) -> Resolution<'a> {
            let answer = match self.0 {
                Some(true) => Ok(vec![SITE_IP.parse().unwrap()]),
                Some(false) => Ok(Vec::new()),
                None => Err(io::ErrorKind::TimedOut.into()),
            };
            Box::pin(async move { answer })
        }
    }

    /// What a fake upstream's tunnels lead to: the resolver and the trace page, answering `trace`, with a certificate
    /// the check trusts or not; or a tunnel closed at once.
    #[derive(Clone, Copy)]
    enum Far {
        Real(&'static str),
        Impostor,
        Closed,
    }

    /// Serves `far` on a tunnel's far end.
    async fn far_end<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
        stream: S,
        far: Far,
        trusted: &Arc<rustls::ServerConfig>,
    ) {
        let (config, trace) = match far {
            Far::Real(trace) => (trusted.clone(), trace),
            Far::Impostor => (certified(&[RESOLVER, SITE]).1, TRACE),
            Far::Closed => return,
        };
        let Ok(mut tls) = tokio_rustls::TlsAcceptor::from(config).accept(stream).await else { return };
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            match tls.read_u8().await {
                Ok(b) => head.push(b),
                // The resolver's tunnel: closed once its handshake went through.
                Err(_) => return,
            }
        }
        assert!(head.starts_with(b"GET /cdn-cgi/trace HTTP/1.0\r\nHost: site.test\r\n"));
        let _ = tls.write_all(trace.as_bytes()).await;
        let _ = tls.shutdown().await;
    }

    /// The check of `url` (PORT replaced by the fake's port), trusting `upstream_cert` for an https upstream and `far`
    /// for the resolver and the site, with names resolving or not, within `deadline`.
    async fn checked(
        url: &str,
        port: u16,
        upstream_cert: Option<CertificateDer<'static>>,
        far: CertificateDer<'static>,
        names: Option<bool>,
        deadline: Duration,
    ) -> Outcome {
        let upstream = Upstream::parse_url(&url.replace("PORT", &port.to_string())).unwrap();
        let dialer = if upstream.scheme == Scheme::Https {
            Dialer::with_tls(upstream, trusting(upstream_cert))
        } else {
            Dialer::new(upstream).unwrap()
        };
        let probe = Probe {
            resolver: (RESOLVER_AT.parse().unwrap(), RESOLVER, trusting(Some(far.clone()))),
            names: Arc::new(Names(names)),
            host: SITE,
            tls: trusting(Some(far)),
            deadline,
        };
        check(&dialer, &probe).await
    }

    /// An HTTP proxy answering every CONNECT with `answer`, then `far` on a 2xx; over TLS with `tls`.
    async fn http_upstream(
        answer: &'static str,
        far: Far,
        tls: Option<Arc<rustls::ServerConfig>>,
    ) -> (u16, CertificateDer<'static>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (cert, trusted) = certified(&[RESOLVER, SITE]);
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let (tls, trusted) = (tls.clone(), trusted.clone());
                tokio::spawn(async move {
                    let mut stream: Boxed = match tls {
                        Some(config) => match tokio_rustls::TlsAcceptor::from(config).accept(tcp).await {
                            Ok(s) => Box::new(s),
                            Err(_) => return,
                        },
                        None => Box::new(tcp),
                    };
                    let mut head = Vec::new();
                    while !head.ends_with(b"\r\n\r\n") {
                        let Ok(b) = stream.read_u8().await else { return };
                        head.push(b);
                    }
                    let to = if head.starts_with(b"CONNECT 198.51.100.53:443 ") { RESOLVER_AT } else { SITE_IP };
                    assert!(head.starts_with(format!("CONNECT {to}").as_bytes()), "{}", String::from_utf8_lossy(&head));
                    stream.write_all(answer.as_bytes()).await.unwrap();
                    if answer.starts_with("HTTP/1.1 200") {
                        far_end(stream, far, &trusted).await;
                    }
                });
            }
        });
        (port, cert)
    }

    /// A SOCKS5 proxy that takes `u`:`password` and answers every CONNECT with `reply`, then `far`.
    async fn socks5_upstream(password: &'static str, reply: u8, far: Far) -> (u16, CertificateDer<'static>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (cert, trusted) = certified(&[RESOLVER, SITE]);
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let trusted = trusted.clone();
                tokio::spawn(async move {
                    let mut greeting = [0u8; 3];
                    s.read_exact(&mut greeting).await.unwrap();
                    // User name and password, or no acceptable method.
                    if greeting != [5, 1, 2] {
                        s.write_all(&[5, 0xff]).await.unwrap();
                        return;
                    }
                    s.write_all(&[5, 2]).await.unwrap();
                    let mut auth = [0u8; 6];
                    s.read_exact(&mut auth).await.unwrap();
                    let right = auth[..] == [&[1, 1, b'u', 2][..], password.as_bytes()].concat();
                    s.write_all(&[1, if right { 0 } else { 1 }]).await.unwrap();
                    if !right {
                        return;
                    }
                    let mut request = [0u8; 10];
                    s.read_exact(&mut request).await.unwrap();
                    assert_eq!(request[..4], [5, 1, 0, 1]);
                    s.write_all(&[5, reply, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
                    if reply == 0 {
                        far_end(s, far, &trusted).await;
                    }
                });
            }
        });
        (port, cert)
    }

    fn reason(outcome: &Outcome) -> Option<Reason> {
        match outcome {
            Outcome::Failed { reason, .. } => Some(*reason),
            Outcome::Passed { .. } => None,
        }
    }

    fn passed(outcome: &Outcome) {
        match outcome {
            Outcome::Passed { ip, country, .. } => {
                assert_eq!((ip.to_string().as_str(), country.as_str()), ("203.0.113.9", "BR"))
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn an_http_upstream_that_reaches_the_trace_page_passes_with_its_exit_and_each_refusal_has_its_reason() {
        let ok = "HTTP/1.1 200 Connection established\r\n\r\n";
        let (port, cert) = http_upstream(ok, Far::Real(TRACE), None).await;
        let outcome = checked("http://u:pw@127.0.0.1:PORT", port, None, cert, Some(true), DEADLINE).await;
        passed(&outcome);
        let shown = outcome.to_json();
        assert!(shown.contains(r#""ip":"203.0.113.9""#) && shown.contains(r#""country":"BR""#), "{shown}");
        let refused_with_body = "HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 4\r\n\r\nnope";
        for (answer, far, url, names, expected) in [
            (refused_with_body, Far::Real(TRACE), "http://u:pw@127.0.0.1:PORT", Some(true), Reason::Auth),
            (
                "HTTP/1.1 407 Proxy Authentication Required\r\n\r\n",
                Far::Real(TRACE),
                "http://127.0.0.1:PORT",
                Some(true),
                Reason::Auth,
            ),
            ("HTTP/1.1 403 Forbidden\r\n\r\n", Far::Real(TRACE), "http://127.0.0.1:PORT", Some(true), Reason::Refused),
            ("SSH-2.0-OpenSSH_9.9\r\n\r\n", Far::Real(TRACE), "http://127.0.0.1:PORT", Some(true), Reason::Protocol),
            ("HTTP/1.1 200 OK\r\n\r\n", Far::Closed, "http://127.0.0.1:PORT", Some(true), Reason::Refused),
            ("HTTP/1.1 200 OK\r\n\r\n", Far::Impostor, "http://127.0.0.1:PORT", Some(true), Reason::Tls),
            ("HTTP/1.1 200 OK\r\n\r\n", Far::Real(TRACE), "http://127.0.0.1:PORT", Some(false), Reason::Refused),
            ("HTTP/1.1 200 OK\r\n\r\n", Far::Real(TRACE), "http://127.0.0.1:PORT", None, Reason::Timeout),
            (
                "HTTP/1.1 200 OK\r\n\r\n",
                Far::Real("HTTP/1.1 200 OK\r\n\r\nhello\n"),
                "http://127.0.0.1:PORT",
                Some(true),
                Reason::Unexpected,
            ),
            (
                "HTTP/1.1 200 OK\r\n\r\n",
                Far::Real("HTTP/1.1 403 Forbidden\r\n\r\nip=1.2.3.4\nloc=BR\n"),
                "http://127.0.0.1:PORT",
                Some(true),
                Reason::Unexpected,
            ),
        ] {
            let (port, cert) = http_upstream(answer, far, None).await;
            let outcome = checked(url, port, None, cert, names, DEADLINE).await;
            assert_eq!(reason(&outcome), Some(expected), "{answer:?} {url} {names:?}: {outcome:?}");
        }
        let (port, cert) = http_upstream("HTTP/1.1 403 Forbidden\r\n\r\n", Far::Real(TRACE), None).await;
        let shown = checked("http://127.0.0.1:PORT", port, None, cert, Some(true), DEADLINE).await.to_json();
        assert_eq!(shown, r#"{"detail":"the proxy refused the tunnel (HTTP 403)","ok":false,"reason":"refused"}"#);
    }

    #[tokio::test]
    async fn an_https_upstream_passes_only_with_a_certificate_for_its_name() {
        let (upstream_cert, config) = certified(&["localhost"]);
        let ok = "HTTP/1.1 200 Connection established\r\n\r\n";
        let (port, cert) = http_upstream(ok, Far::Real(TRACE), Some(config.clone())).await;
        passed(
            &checked("https://u:pw@localhost:PORT", port, Some(upstream_cert.clone()), cert, Some(true), DEADLINE)
                .await,
        );
        let (port, cert) = http_upstream(ok, Far::Real(TRACE), Some(config)).await;
        let outcome = checked("https://u:pw@localhost:PORT", port, None, cert, Some(true), DEADLINE).await;
        assert_eq!(outcome, failed(Reason::Tls, "the proxy's certificate did not verify for its name"));
        // A plain HTTP proxy named https: it answers the TLS greeting as a bad request.
        let plain = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = plain.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = plain.accept().await.unwrap();
            let _ = s.write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n").await;
        });
        let (cert, _) = certified(&[SITE]);
        let outcome = checked("https://localhost:PORT", port, Some(upstream_cert), cert, Some(true), DEADLINE).await;
        assert_eq!(outcome, failed(Reason::Tls, "the TLS handshake with the proxy failed"));
    }

    #[tokio::test]
    async fn a_socks5_upstream_passes_with_its_credentials_and_each_refusal_has_its_reason() {
        let (port, cert) = socks5_upstream("pw", 0, Far::Real(TRACE)).await;
        passed(&checked("socks5://u:pw@127.0.0.1:PORT", port, None, cert, Some(true), DEADLINE).await);
        let (port, cert) = socks5_upstream("pw", 0, Far::Real(TRACE)).await;
        let outcome = checked("socks5://u:zq@127.0.0.1:PORT", port, None, cert, Some(true), DEADLINE).await;
        assert_eq!(outcome, failed(Reason::Auth, "the proxy refused the user name or password"));
        assert!(!outcome.to_json().contains("zq"), "nothing given is repeated");
        let (port, cert) = socks5_upstream("pw", 2, Far::Real(TRACE)).await;
        let outcome = checked("socks5://u:pw@127.0.0.1:PORT", port, None, cert, Some(true), DEADLINE).await;
        assert_eq!(outcome, failed(Reason::Refused, "the proxy refused the tunnel (SOCKS5 reply 2)"));
        // Without credentials, an upstream that wants them.
        let (port, cert) = socks5_upstream("pw", 0, Far::Real(TRACE)).await;
        let outcome = checked("socks5://127.0.0.1:PORT", port, None, cert, Some(true), DEADLINE).await;
        assert_eq!(outcome, failed(Reason::Auth, "the proxy asks for a user name and password"));
    }

    #[tokio::test]
    async fn an_upstream_that_does_not_answer_is_unreachable_or_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed = listener.local_addr().unwrap().port();
        drop(listener);
        let (cert, _) = certified(&[SITE]);
        let outcome = checked("http://127.0.0.1:PORT", closed, None, cert.clone(), Some(true), DEADLINE).await;
        assert_eq!(outcome, failed(Reason::Unreachable, "nothing listens at the proxy's address and port"));
        // One that accepts and never answers.
        let silent = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = silent.local_addr().unwrap().port();
        let held = tokio::spawn(async move {
            let _accepted = silent.accept().await;
            std::future::pending::<()>().await;
        });
        let outcome =
            checked("socks5://127.0.0.1:PORT", port, None, cert, Some(true), Duration::from_millis(200)).await;
        held.abort();
        match outcome {
            Outcome::Failed { reason: Reason::Timeout, detail } => {
                assert!(detail.starts_with("the proxy did not open a tunnel within"), "{detail}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_trace_names_its_address_and_country_or_nothing() {
        let parsed = trace(TRACE.as_bytes()).unwrap();
        assert_eq!((parsed.0.to_string(), parsed.1), ("203.0.113.9".to_owned(), "BR".to_owned()));
        let v6 = "HTTP/1.0 200 OK\r\nContent-Length: 24\r\n\r\nip=2001:db8::9\nloc=DE\nfl=1\n";
        assert_eq!(trace(v6.as_bytes()).unwrap(), ("2001:db8::9".parse().unwrap(), "DE".to_owned()));
        // Chunked framing could split a line: it is refused rather than read through.
        let chunked = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nf\r\nip=203.0.113.9\r\n1\r\n9\r\n";
        assert!(trace(chunked.as_bytes()).is_none());
        for bad in [
            "HTTP/1.1 200 OK\r\n\r\nip=nonsense\nloc=BR\n",
            "HTTP/1.1 200 OK\r\n\r\nip=203.0.113.9\nloc=Brazil\n",
            "HTTP/1.1 200 OK\r\n\r\nip=203.0.113.9\n",
            "HTTP/1.1 302 Found\r\n\r\nip=203.0.113.9\nloc=BR\n",
            "ip=203.0.113.9\nloc=BR\n",
        ] {
            assert!(trace(bad.as_bytes()).is_none(), "{bad:?}");
        }
    }
}
