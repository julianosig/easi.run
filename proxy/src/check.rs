//! The check of an upstream before sessions leave through it (`PROXY_CHECK=1`), run by the controller in a container of
//! its own on a network like a session proxy's, so the upstream is judged from where a session will reach it. It reaches
//! the upstream as a session's proxy does (the system resolver for its name, TLS verified against the public roots for
//! an `https` one) and goes through it as a session's first page does: a tunnel to the DNS-over-HTTPS resolver, by
//! address, with its TLS verified as `doh` verifies it; the speed test's name resolved over that tunnel by `doh`
//! itself; and, through a tunnel to the vetted address it gave, the metadata Cloudflare's speed test reads fetched over
//! verified TLS (`GET https://speed.cloudflare.com/meta`), which says the address the request came from and where that
//! address is: its country and, when known, its region and city. All of it within [`DEADLINE`].
//!
//! The outcome is one JSON line (see [`egress::check`]), whose detail, when it failed, is a fixed sentence (at most with
//! a status number), never anything the upstream or the page sent, or the check was given.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egress::check::{CheckOutcome, CheckReason, Exit};
use egress::{Scheme, country_code, place};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::resolve::Resolver;
use crate::upstream::{Answer, Boxed, Dialer, Target, tls, tls_config};

/// How long the whole check may take: two tunnels through the upstream, one DNS query on the first.
pub const DEADLINE: Duration = Duration::from_secs(10);
/// The speed test's host, and the path of the metadata it reads (which answers only a request from its page).
const META_HOST: &str = "speed.cloudflare.com";
const META_PATH: &str = "/meta";
/// The most of the metadata's answer read.
const MAX_META: usize = 16 * 1024;

fn failed(reason: CheckReason, detail: impl Into<String>) -> CheckOutcome {
    CheckOutcome::Failed { reason, detail: detail.into() }
}

/// What the check goes through the upstream to: in production, the resolver `doh` uses and Cloudflare's speed test.
pub struct Probe {
    /// The DNS-over-HTTPS resolver's address, the name its certificate is verified for, and its TLS.
    pub resolver: (SocketAddr, &'static str, Arc<rustls::ClientConfig>),
    /// What resolves the speed test's name: `doh` through the upstream.
    pub names: Arc<dyn Resolver>,
    /// That `doh`, which takes the resolver's connection the check makes as its own.
    pub doh: Option<Arc<crate::doh::Doh>>,
    /// The speed test's host, whose certificate is verified, and its TLS.
    pub host: &'static str,
    pub tls: Arc<rustls::ClientConfig>,
    pub deadline: Duration,
}

impl Probe {
    /// The check every upstream gets: through `dialer`'s upstream to the resolver every session's names go to, and to
    /// Cloudflare's speed test by the address that resolver gives.
    pub fn new(dialer: Arc<Dialer>) -> io::Result<Probe> {
        let doh = Arc::new(crate::doh::Doh::new(dialer)?);
        Ok(Probe {
            resolver: (crate::doh::SERVER, crate::doh::SERVER_NAME, tls_config(&[b"h2"])?),
            names: doh.clone(),
            doh: Some(doh),
            host: META_HOST,
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
pub async fn check(dialer: &Dialer, probe: &Probe) -> CheckOutcome {
    let started = Instant::now();
    let mut step = Step::Resolve;
    let seconds = probe.deadline.as_secs();
    match tokio::time::timeout(probe.deadline, steps(dialer, probe, &mut step)).await {
        Ok(Ok(exit)) => CheckOutcome::Passed { latency_ms: started.elapsed().as_millis() as u64, exit },
        Ok(Err(outcome)) => outcome,
        Err(_) => failed(
            CheckReason::Timeout,
            match step {
                Step::Resolve => format!("the proxy's name did not resolve within {seconds} s"),
                Step::Connect => format!("the proxy did not answer within {seconds} s"),
                Step::Handshake => format!("the TLS handshake with the proxy did not finish within {seconds} s"),
                Step::Tunnel => format!("the proxy did not open a tunnel within {seconds} s"),
                Step::Names => format!("names did not resolve through the proxy within {seconds} s"),
                Step::Site => format!("the speed test did not answer through the proxy within {seconds} s"),
            },
        ),
    }
}

async fn steps(dialer: &Dialer, probe: &Probe, step: &mut Step) -> Result<Exit, CheckOutcome> {
    let upstream = dialer.upstream();
    let (host, port) = (upstream.host.as_str(), upstream.port);
    let unresolved = || failed(CheckReason::Unreachable, "the proxy's name does not resolve");
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
        return Err(failed(CheckReason::Unreachable, detail));
    };
    let _ = tcp.set_nodelay(true);
    let stream: Boxed = match dialer.tls() {
        Some(config) => {
            *step = Step::Handshake;
            tls(Box::new(tcp), host, config).await.map_err(|e| match certificate(&e) {
                Some(true) => failed(CheckReason::Tls, "the proxy's certificate did not verify for its name"),
                _ => failed(CheckReason::Tls, "the TLS handshake with the proxy failed"),
            })?
        }
        None => Box::new(tcp),
    };
    // The resolver's tunnel first, on this connection: what fails there fails every session's names.
    *step = Step::Tunnel;
    let (resolver, resolver_name, resolver_tls) = &probe.resolver;
    let tunnel = dialer.through(stream, Target::Addr(*resolver)).await.map_err(|e| tunnel_failure(dialer, &e))?;
    *step = Step::Site;
    let resolver_stream = tls(tunnel, resolver_name, resolver_tls.clone()).await.map_err(|e| far_failure(&e))?;
    *step = Step::Names;
    if let Some(doh) = &probe.doh {
        doh.adopt(resolver_stream).await.map_err(|e| names_failure(dialer, &e))?;
    }
    let names = probe.names.resolve(probe.host, 443).await.map_err(|e| names_failure(dialer, &e))?;
    let addr = names.into_iter().find(|a| egress::destination::classify(*a).is_ok());
    let addr = addr.ok_or_else(|| failed(CheckReason::Refused, "names did not resolve through the proxy"))?;
    *step = Step::Tunnel;
    let to = SocketAddr::new(egress::destination::dial_address(addr), 443);
    let tunnel = dialer.dial(Target::Addr(to)).await.map_err(|e| tunnel_failure(dialer, &e))?;
    *step = Step::Site;
    let mut page = tls(tunnel, probe.host, probe.tls.clone()).await.map_err(|e| far_failure(&e))?;
    let request = format!(
        // HTTP/1.0: the answer is the body itself up to the close, never chunked.
        "GET {META_PATH} HTTP/1.0\r\nHost: {0}\r\nAccept: application/json\r\nReferer: https://{0}/\r\nUser-Agent: easi.run\r\nConnection: close\r\n\r\n",
        probe.host
    );
    let closed = || failed(CheckReason::Refused, "the proxy closed the tunnel before the speed test answered");
    page.write_all(request.as_bytes()).await.map_err(|_| closed())?;
    page.flush().await.map_err(|_| closed())?;
    let mut answer = Vec::new();
    let mut buf = [0u8; 4096];
    // To its end (a close without TLS's own close message included), or the most read.
    while answer.len() < MAX_META {
        match page.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => answer.extend_from_slice(&buf[..n]),
        }
    }
    if answer.is_empty() {
        return Err(closed());
    }
    meta(&answer).ok_or_else(|| failed(CheckReason::Unexpected, "the speed test's answer is not its metadata"))
}

/// Where the metadata's answer says the request came from: a 200 to the HTTP/1.0 request, not chunked, whose JSON body
/// names `clientIp` and `country`, and maybe `region` and `city`, which are kept only when they are place names.
fn meta(answer: &[u8]) -> Option<Exit> {
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
    #[derive(serde::Deserialize)]
    struct Meta {
        #[serde(rename = "clientIp")]
        ip: IpAddr,
        country: String,
        #[serde(default)]
        region: Option<String>,
        #[serde(default)]
        city: Option<String>,
    }
    let Meta { ip, country, region, city } = serde_json::from_str(body).ok()?;
    country_code(&country).then(|| Exit {
        ip,
        country,
        region: region.filter(|r| place(r)),
        city: city.filter(|c| place(c)),
    })
}

/// Whether a TLS failure is TLS's own (`Some`), and then whether it is the certificate's; `None` for the connection's.
fn certificate(e: &io::Error) -> Option<bool> {
    let tls = e.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>())?;
    Some(matches!(tls, rustls::Error::InvalidCertificate(_)))
}

/// A TLS handshake through a tunnel that failed.
fn far_failure(e: &io::Error) -> CheckOutcome {
    match certificate(e) {
        // The far end is not who it should be: possibly an upstream that intercepts TLS.
        Some(true) => failed(CheckReason::Tls, "a certificate did not verify through the proxy"),
        Some(false) => failed(CheckReason::Tls, "a TLS handshake through the proxy failed"),
        None => failed(CheckReason::Refused, "the proxy closed a tunnel before the far end answered"),
    }
}

/// A lookup through the upstream's resolver that failed: the tunnel's refusal, the resolver's TLS, its deadline, or
/// its answers.
fn names_failure(dialer: &Dialer, e: &io::Error) -> CheckOutcome {
    if e.get_ref().is_some_and(|inner| inner.is::<Answer>()) {
        return tunnel_failure(dialer, e);
    }
    if certificate(e).is_some() {
        return far_failure(e);
    }
    if e.kind() == io::ErrorKind::TimedOut {
        return failed(CheckReason::Timeout, "names did not resolve through the proxy in time");
    }
    failed(CheckReason::Refused, "names did not resolve through the proxy")
}

fn tunnel_failure(dialer: &Dialer, e: &io::Error) -> CheckOutcome {
    let upstream = dialer.upstream();
    let answer = e.get_ref().and_then(|inner| inner.downcast_ref::<Answer>());
    match answer {
        Some(Answer::Auth) if upstream.username.is_some() => {
            failed(CheckReason::Auth, "the proxy refused the user name or password")
        }
        Some(Answer::Auth) => failed(CheckReason::Auth, "the proxy asks for a user name and password"),
        Some(Answer::Status(code)) => {
            failed(CheckReason::Refused, format!("the proxy refused the tunnel (HTTP {code})"))
        }
        Some(Answer::Reply(code)) => {
            failed(CheckReason::Refused, format!("the proxy refused the tunnel (SOCKS5 reply {code})"))
        }
        Some(Answer::Malformed) => failed(
            CheckReason::Protocol,
            match upstream.scheme {
                Scheme::Http | Scheme::Https => "the proxy does not answer as an HTTP proxy",
                Scheme::Socks5 => "the proxy does not answer as a SOCKS5 proxy",
            },
        ),
        None if e.kind() == io::ErrorKind::ConnectionRefused => {
            failed(CheckReason::Unreachable, "nothing listens at the proxy's address and port")
        }
        None => failed(CheckReason::Protocol, "the proxy closed the connection before answering"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::Resolution;
    use crate::upstream::fixtures::{certified, trusting};
    use egress::Upstream;
    use rustls::pki_types::CertificateDer;
    use tokio::net::TcpListener;

    /// The resolver's and the speed test's names in these tests, which one certificate covers, and their addresses.
    const RESOLVER: &str = "resolver.test";
    const SITE: &str = "site.test";
    const RESOLVER_AT: &str = "198.51.100.53:443";
    const SITE_IP: &str = "93.184.216.34";
    const META: &str = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"clientIp\":\"203.0.113.9\",\"country\":\"BR\",\"region\":\"São Paulo\",\"city\":\"São Paulo\",\"asn\":1,\"colo\":{\"iata\":\"GRU\"}}";

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

    /// What a fake upstream's tunnels lead to: the resolver and the speed test, answering `trace`, with a certificate
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
            Far::Impostor => (certified(&[RESOLVER, SITE]).1, META),
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
        assert!(head.starts_with(b"GET /meta HTTP/1.0\r\nHost: site.test\r\n"));
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
    ) -> CheckOutcome {
        let upstream = Upstream::parse_url(&url.replace("PORT", &port.to_string())).unwrap();
        let dialer = if upstream.scheme == Scheme::Https {
            Dialer::with_tls(upstream, trusting(upstream_cert))
        } else {
            Dialer::new(upstream).unwrap()
        };
        let probe = Probe {
            resolver: (RESOLVER_AT.parse().unwrap(), RESOLVER, trusting(Some(far.clone()))),
            names: Arc::new(Names(names)),
            doh: None,
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

    fn reason(outcome: &CheckOutcome) -> Option<CheckReason> {
        match outcome {
            CheckOutcome::Failed { reason, .. } => Some(*reason),
            CheckOutcome::Passed { .. } => None,
        }
    }

    fn passed(outcome: &CheckOutcome) {
        match outcome {
            CheckOutcome::Passed { exit, .. } => assert_eq!(
                (exit.ip.to_string().as_str(), exit.country.as_str(), exit.city.as_deref()),
                ("203.0.113.9", "BR", Some("São Paulo"))
            ),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn an_http_upstream_that_reaches_the_trace_page_passes_with_its_exit_and_each_refusal_has_its_reason() {
        let ok = "HTTP/1.1 200 Connection established\r\n\r\n";
        let (port, cert) = http_upstream(ok, Far::Real(META), None).await;
        let outcome = checked("http://u:pw@127.0.0.1:PORT", port, None, cert, Some(true), DEADLINE).await;
        passed(&outcome);
        let shown = serde_json::to_string(&outcome).unwrap();
        assert!(shown.contains(r#""ip":"203.0.113.9""#) && shown.contains(r#""country":"BR""#), "{shown}");
        let refused_with_body = "HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 4\r\n\r\nnope";
        for (answer, far, url, names, expected) in [
            (refused_with_body, Far::Real(META), "http://u:pw@127.0.0.1:PORT", Some(true), CheckReason::Auth),
            (
                "HTTP/1.1 407 Proxy Authentication Required\r\n\r\n",
                Far::Real(META),
                "http://127.0.0.1:PORT",
                Some(true),
                CheckReason::Auth,
            ),
            (
                "HTTP/1.1 403 Forbidden\r\n\r\n",
                Far::Real(META),
                "http://127.0.0.1:PORT",
                Some(true),
                CheckReason::Refused,
            ),
            (
                "SSH-2.0-OpenSSH_9.9\r\n\r\n",
                Far::Real(META),
                "http://127.0.0.1:PORT",
                Some(true),
                CheckReason::Protocol,
            ),
            ("HTTP/1.1 200 OK\r\n\r\n", Far::Closed, "http://127.0.0.1:PORT", Some(true), CheckReason::Refused),
            ("HTTP/1.1 200 OK\r\n\r\n", Far::Impostor, "http://127.0.0.1:PORT", Some(true), CheckReason::Tls),
            ("HTTP/1.1 200 OK\r\n\r\n", Far::Real(META), "http://127.0.0.1:PORT", Some(false), CheckReason::Refused),
            ("HTTP/1.1 200 OK\r\n\r\n", Far::Real(META), "http://127.0.0.1:PORT", None, CheckReason::Timeout),
            (
                "HTTP/1.1 200 OK\r\n\r\n",
                Far::Real("HTTP/1.1 200 OK\r\n\r\nhello\n"),
                "http://127.0.0.1:PORT",
                Some(true),
                CheckReason::Unexpected,
            ),
            (
                "HTTP/1.1 200 OK\r\n\r\n",
                Far::Real("HTTP/1.1 403 Forbidden\r\n\r\n{\"clientIp\":\"1.2.3.4\",\"country\":\"BR\"}"),
                "http://127.0.0.1:PORT",
                Some(true),
                CheckReason::Unexpected,
            ),
        ] {
            let (port, cert) = http_upstream(answer, far, None).await;
            let outcome = checked(url, port, None, cert, names, DEADLINE).await;
            assert_eq!(reason(&outcome), Some(expected), "{answer:?} {url} {names:?}: {outcome:?}");
        }
        let (port, cert) = http_upstream("HTTP/1.1 403 Forbidden\r\n\r\n", Far::Real(META), None).await;
        let outcome = checked("http://127.0.0.1:PORT", port, None, cert, Some(true), DEADLINE).await;
        let shown = serde_json::to_string(&outcome).unwrap();
        assert_eq!(shown, r#"{"ok":false,"reason":"refused","detail":"the proxy refused the tunnel (HTTP 403)"}"#);
    }

    #[tokio::test]
    async fn an_https_upstream_passes_only_with_a_certificate_for_its_name() {
        let (upstream_cert, config) = certified(&["localhost"]);
        let ok = "HTTP/1.1 200 Connection established\r\n\r\n";
        let (port, cert) = http_upstream(ok, Far::Real(META), Some(config.clone())).await;
        passed(
            &checked("https://u:pw@localhost:PORT", port, Some(upstream_cert.clone()), cert, Some(true), DEADLINE)
                .await,
        );
        let (port, cert) = http_upstream(ok, Far::Real(META), Some(config)).await;
        let outcome = checked("https://u:pw@localhost:PORT", port, None, cert, Some(true), DEADLINE).await;
        assert_eq!(outcome, failed(CheckReason::Tls, "the proxy's certificate did not verify for its name"));
        // A plain HTTP proxy named https: it answers the TLS greeting as a bad request.
        let plain = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = plain.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = plain.accept().await.unwrap();
            let _ = s.write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n").await;
        });
        let (cert, _) = certified(&[SITE]);
        let outcome = checked("https://localhost:PORT", port, Some(upstream_cert), cert, Some(true), DEADLINE).await;
        assert_eq!(outcome, failed(CheckReason::Tls, "the TLS handshake with the proxy failed"));
    }

    #[tokio::test]
    async fn a_socks5_upstream_passes_with_its_credentials_and_each_refusal_has_its_reason() {
        let (port, cert) = socks5_upstream("pw", 0, Far::Real(META)).await;
        passed(&checked("socks5://u:pw@127.0.0.1:PORT", port, None, cert, Some(true), DEADLINE).await);
        let (port, cert) = socks5_upstream("pw", 0, Far::Real(META)).await;
        let outcome = checked("socks5://u:zq@127.0.0.1:PORT", port, None, cert, Some(true), DEADLINE).await;
        assert_eq!(outcome, failed(CheckReason::Auth, "the proxy refused the user name or password"));
        assert!(!serde_json::to_string(&outcome).unwrap().contains("zq"), "nothing given is repeated");
        let (port, cert) = socks5_upstream("pw", 2, Far::Real(META)).await;
        let outcome = checked("socks5://u:pw@127.0.0.1:PORT", port, None, cert, Some(true), DEADLINE).await;
        assert_eq!(outcome, failed(CheckReason::Refused, "the proxy refused the tunnel (SOCKS5 reply 2)"));
        // Without credentials, an upstream that wants them.
        let (port, cert) = socks5_upstream("pw", 0, Far::Real(META)).await;
        let outcome = checked("socks5://127.0.0.1:PORT", port, None, cert, Some(true), DEADLINE).await;
        assert_eq!(outcome, failed(CheckReason::Auth, "the proxy asks for a user name and password"));
    }

    #[tokio::test]
    async fn an_upstream_that_does_not_answer_is_unreachable_or_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed = listener.local_addr().unwrap().port();
        drop(listener);
        let (cert, _) = certified(&[SITE]);
        let outcome = checked("http://127.0.0.1:PORT", closed, None, cert.clone(), Some(true), DEADLINE).await;
        assert_eq!(outcome, failed(CheckReason::Unreachable, "nothing listens at the proxy's address and port"));
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
            CheckOutcome::Failed { reason: CheckReason::Timeout, detail } => {
                assert!(detail.starts_with("the proxy did not open a tunnel within"), "{detail}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_metadata_names_its_address_and_place_or_nothing() {
        let exit = meta(META.as_bytes()).unwrap();
        assert_eq!(
            exit,
            Exit {
                ip: "203.0.113.9".parse().unwrap(),
                country: "BR".into(),
                region: Some("São Paulo".into()),
                city: Some("São Paulo".into())
            }
        );
        let v6 = "HTTP/1.0 200 OK\r\n\r\n{\"clientIp\":\"2001:db8::9\",\"country\":\"DE\"}";
        let exit = meta(v6.as_bytes()).unwrap();
        assert_eq!((exit.ip, exit.region, exit.city), ("2001:db8::9".parse().unwrap(), None, None));
        // A region or city that is no place name is left out, the rest kept.
        let odd = "HTTP/1.1 200 OK\r\n\r\n{\"clientIp\":\"203.0.113.9\",\"country\":\"BR\",\"region\":\"<b>x</b>\",\"city\":\"Rio de Janeiro\"}";
        let exit = meta(odd.as_bytes()).unwrap();
        assert_eq!((exit.region, exit.city.as_deref()), (None, Some("Rio de Janeiro")));
        // Chunked framing would have to be decoded first: it is refused rather than read through.
        let chunked =
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{\"clientIp\":\"203.0.113.9\",\"country\":\"BR\"}";
        assert!(meta(chunked.as_bytes()).is_none());
        for bad in [
            "HTTP/1.1 200 OK\r\n\r\n{\"clientIp\":\"nonsense\",\"country\":\"BR\"}",
            "HTTP/1.1 200 OK\r\n\r\n{\"clientIp\":\"203.0.113.9\",\"country\":\"Brazil\"}",
            "HTTP/1.1 200 OK\r\n\r\n{\"clientIp\":\"203.0.113.9\"}",
            "HTTP/1.1 200 OK\r\n\r\n{}",
            "HTTP/1.1 302 Found\r\n\r\n{\"clientIp\":\"203.0.113.9\",\"country\":\"BR\"}",
            "{\"clientIp\":\"203.0.113.9\",\"country\":\"BR\"}",
        ] {
            assert!(meta(bad.as_bytes()).is_none(), "{bad:?}");
        }
    }
}
