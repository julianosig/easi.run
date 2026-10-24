//! Dialling through the deployment's upstream proxy (the egress crate's `Upstream`): HTTP `CONNECT` over TCP or over
//! verified TLS, or SOCKS5. The upstream is the operator's own endpoint, so its address is resolved by the system
//! resolver; every destination asked of it has already been vetted.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use base64::Engine as _;
use egress::{Scheme, Upstream};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

/// The largest response head an HTTP upstream may answer `CONNECT` with.
const MAX_HEAD: usize = 16 * 1024;

/// A connection, plain or TLS.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}
pub type Boxed = Box<dyn Stream>;

/// What the upstream is asked to connect to: a vetted address, or (in the bench profile only) the fixture host by name,
/// which the test upstream maps itself.
#[derive(Clone, Copy, Debug)]
pub enum Target<'a> {
    Addr(SocketAddr),
    Name(&'a str, u16),
}

impl Target<'_> {
    /// `host:port` as an HTTP `CONNECT` names it.
    fn authority(&self) -> String {
        match self {
            Target::Addr(addr) => addr.to_string(),
            Target::Name(name, port) => format!("{name}:{port}"),
        }
    }
}

/// The public roots both the upstream's TLS and the DNS-over-HTTPS resolver verify against.
pub fn tls_config(alpn: &[&[u8]]) -> io::Result<Arc<rustls::ClientConfig>> {
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Ok(Arc::new(config))
}

/// Wraps `stream` in TLS to `host`, verifying its certificate for that name or address.
pub async fn tls(stream: Boxed, host: &str, config: Arc<rustls::ClientConfig>) -> io::Result<Boxed> {
    let name = ServerName::try_from(host.to_owned()).map_err(|_| io::Error::other("not a TLS server name"))?;
    let connector = tokio_rustls::TlsConnector::from(config);
    Ok(Box::new(connector.connect(name, stream).await?))
}

/// The upstream and what reaching it needs.
pub struct Dialer {
    upstream: Upstream,
    tls: Option<Arc<rustls::ClientConfig>>,
}

impl Dialer {
    pub fn new(upstream: Upstream) -> io::Result<Self> {
        upstream.validate().map_err(io::Error::other)?;
        let tls = (upstream.scheme == Scheme::Https).then(|| tls_config(&[])).transpose()?;
        Ok(Self { upstream, tls })
    }

    /// A stream to `target` through the upstream, ready to carry the client's bytes.
    pub async fn dial(&self, target: Target<'_>) -> io::Result<Boxed> {
        let stream = self.reach().await?;
        match self.upstream.scheme {
            Scheme::Http | Scheme::Https => connect(stream, target, self.upstream.basic_credentials()).await,
            Scheme::Socks5 => socks5(stream, target, &self.upstream).await,
        }
    }

    /// The connection to the upstream itself, over TLS for an `https` one.
    async fn reach(&self) -> io::Result<Boxed> {
        let (host, port) = (self.upstream.host.as_str(), self.upstream.port);
        let tcp = TcpStream::connect((host, port)).await?;
        let _ = tcp.set_nodelay(true);
        match &self.tls {
            Some(config) => tls(Box::new(tcp), host, config.clone()).await,
            None => Ok(Box::new(tcp)),
        }
    }
}

/// HTTP `CONNECT`: the tunnel is ready once the upstream answers 2xx; nothing may follow its response head, since
/// every protocol the proxy carries (HTTP, TLS) speaks first from the client.
async fn connect(mut stream: Boxed, target: Target<'_>, credentials: Option<String>) -> io::Result<Boxed> {
    let authority = target.authority();
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(credentials) = credentials {
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
        request.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut head = Vec::with_capacity(256);
    let mut buf = [0u8; 1024];
    let end = loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the upstream closed before answering"));
        }
        head.extend_from_slice(&buf[..n]);
        if let Some(at) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        if head.len() > MAX_HEAD {
            return Err(io::Error::other("the upstream's answer is too large"));
        }
    };
    if end != head.len() {
        return Err(io::Error::other("the upstream sent data before the client"));
    }
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut response = httparse::Response::new(&mut headers);
    match response.parse(&head) {
        Ok(httparse::Status::Complete(_)) if response.code.is_some_and(|c| (200..300).contains(&c)) => Ok(stream),
        Ok(httparse::Status::Complete(_)) => Err(io::Error::other("the upstream refused the tunnel")),
        _ => Err(io::Error::other("the upstream's answer is malformed")),
    }
}

/// SOCKS5 (RFC 1928): no authentication, or user name and password (RFC 1929) when the upstream has credentials.
async fn socks5(mut stream: Boxed, target: Target<'_>, upstream: &Upstream) -> io::Result<Boxed> {
    const USER_PASSWORD: u8 = 2;
    let method = if upstream.username.is_some() { USER_PASSWORD } else { 0 };
    stream.write_all(&[5, 1, method]).await?;
    let mut chosen = [0u8; 2];
    stream.read_exact(&mut chosen).await?;
    if chosen != [5, method] {
        return Err(io::Error::other("the upstream refused the authentication method"));
    }
    if method == USER_PASSWORD {
        let (user, password) = (upstream.username.as_deref().unwrap_or_default(), upstream.password.as_deref());
        let password = password.unwrap_or_default();
        let mut auth = vec![1, user.len() as u8];
        auth.extend_from_slice(user.as_bytes());
        auth.push(password.len() as u8);
        auth.extend_from_slice(password.as_bytes());
        stream.write_all(&auth).await?;
        let mut status = [0u8; 2];
        stream.read_exact(&mut status).await?;
        if status != [1, 0] {
            return Err(io::Error::other("the upstream refused the credentials"));
        }
    }
    let mut request = vec![5, 1, 0];
    let port = match target {
        Target::Addr(SocketAddr::V4(addr)) => {
            request.push(1);
            request.extend_from_slice(&addr.ip().octets());
            addr.port()
        }
        Target::Addr(SocketAddr::V6(addr)) => {
            request.push(4);
            request.extend_from_slice(&addr.ip().octets());
            addr.port()
        }
        Target::Name(name, port) => {
            request.extend_from_slice(&[3, name.len() as u8]);
            request.extend_from_slice(name.as_bytes());
            port
        }
    };
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await?;
    let mut reply = [0u8; 4];
    stream.read_exact(&mut reply).await?;
    if reply[0] != 5 || reply[1] != 0 {
        return Err(io::Error::other("the upstream refused the connection"));
    }
    let bound = match reply[3] {
        1 => 4,
        4 => 16,
        3 => usize::from(stream.read_u8().await?),
        _ => return Err(io::Error::other("the upstream's answer is malformed")),
    };
    let mut rest = vec![0u8; bound + 2];
    stream.read_exact(&mut rest).await?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// What a fake upstream reads (that many bytes) and answers, in turn.
    type Steps = Vec<(usize, Vec<u8>)>;

    /// A one-connection upstream that, for each step, reads that many bytes and answers with that reply, then echoes;
    /// it returns every byte it read.
    async fn fake(steps: Steps) -> (u16, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut got = Vec::new();
            for (len, reply) in steps {
                let mut part = vec![0u8; len];
                if s.read_exact(&mut part).await.is_err() {
                    return got;
                }
                got.extend(part);
                s.write_all(&reply).await.unwrap();
            }
            let mut echo = [0u8; 4];
            if s.read_exact(&mut echo).await.is_ok() {
                s.write_all(&echo).await.unwrap();
            }
            got
        });
        (port, task)
    }

    fn upstream(url: &str, port: u16) -> Dialer {
        Dialer::new(Upstream::parse_url(&url.replace("PORT", &port.to_string())).unwrap()).unwrap()
    }

    async fn carries(mut stream: Boxed) {
        stream.write_all(b"ping").await.unwrap();
        let mut back = [0u8; 4];
        stream.read_exact(&mut back).await.unwrap();
        assert_eq!(&back, b"ping");
    }

    #[tokio::test]
    async fn http_connect_with_basic_credentials() {
        let target: SocketAddr = "93.184.216.34:443".parse().unwrap();
        let expect = b"CONNECT 93.184.216.34:443 HTTP/1.1\r\nHost: 93.184.216.34:443\r\n\
                       Proxy-Authorization: Basic dTpwQHNz\r\n\r\n"
            .to_vec();
        let (port, task) = fake(vec![(expect.len(), b"HTTP/1.1 200 Connection established\r\n\r\n".to_vec())]).await;
        let stream = upstream("http://u:p%40ss@127.0.0.1:PORT", port).dial(Target::Addr(target)).await.unwrap();
        carries(stream).await;
        assert_eq!(task.await.unwrap(), expect);
    }

    #[tokio::test]
    async fn http_refusals_and_early_data_fail() {
        let target = Target::Addr("93.184.216.34:443".parse().unwrap());
        let len = b"CONNECT 93.184.216.34:443 HTTP/1.1\r\nHost: 93.184.216.34:443\r\n\r\n".len();
        for reply in [
            &b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n"[..],
            b"HTTP/1.1 200 OK\r\n\r\nearly",
            b"junk\r\n\r\n",
        ] {
            let (port, _) = fake(vec![(len, reply.to_vec())]).await;
            assert!(upstream("http://127.0.0.1:PORT", port).dial(target).await.is_err(), "{reply:?}");
        }
    }

    #[tokio::test]
    async fn socks5_with_credentials_by_address_and_by_name() {
        let target: SocketAddr = "[2001:db8::7]:443".parse().unwrap();
        let mut request = vec![5, 1, 0, 4];
        request.extend_from_slice(&"2001:db8::7".parse::<std::net::Ipv6Addr>().unwrap().octets());
        request.extend_from_slice(&443u16.to_be_bytes());
        let steps = vec![(3, vec![5, 2]), (6, vec![1, 0]), (request.len(), vec![5, 0, 0, 1, 0, 0, 0, 0, 0, 0])];
        let (port, task) = fake(steps).await;
        let stream = upstream("socks5://u:pw@127.0.0.1:PORT", port).dial(Target::Addr(target)).await.unwrap();
        carries(stream).await;
        let expect = [&[5, 1, 2, 1, 1, b'u', 2, b'p', b'w'][..], &request].concat();
        assert_eq!(task.await.unwrap(), expect);
        let mut by_name = vec![5, 1, 0, 3, 13];
        by_name.extend_from_slice(b"fixture.bench");
        by_name.extend_from_slice(&443u16.to_be_bytes());
        let steps = vec![(3, vec![5, 0]), (by_name.len(), vec![5, 0, 0, 3, 1, b'x', 1, 187])];
        let (port, task) = fake(steps).await;
        let stream = upstream("socks5://127.0.0.1:PORT", port).dial(Target::Name("fixture.bench", 443)).await.unwrap();
        carries(stream).await;
        assert_eq!(task.await.unwrap(), [&[5, 1, 0][..], &by_name].concat());
    }

    #[tokio::test]
    async fn socks5_refusals_fail() {
        let target = Target::Addr("93.184.216.34:443".parse().unwrap());
        // No acceptable method; wrong credentials; a refused connection.
        let cases: [(&str, Steps); 3] = [
            ("socks5://127.0.0.1:PORT", vec![(3, vec![5, 0xff])]),
            ("socks5://u:pw@127.0.0.1:PORT", vec![(3, vec![5, 2]), (6, vec![1, 1])]),
            ("socks5://127.0.0.1:PORT", vec![(3, vec![5, 0]), (10, vec![5, 5, 0, 1, 0, 0, 0, 0, 0, 0])]),
        ];
        for (url, steps) in cases {
            let (port, _) = fake(steps).await;
            assert!(upstream(url, port).dial(target).await.is_err(), "{url}");
        }
    }
}
