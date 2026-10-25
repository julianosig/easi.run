//! Reading and parsing one bounded request head from a hostile client.

use std::net::IpAddr;
use std::time::Duration;

use egress::destination::validate_name;
use tokio::io::{AsyncRead, AsyncReadExt};

/// The largest request head the proxy reads, terminator included.
pub const MAX_HEAD: usize = 16 * 1024;
const MAX_HEADERS: usize = 100;

/// The methods forwarded as absolute-form plain HTTP.
const PLAIN_METHODS: [&str; 7] = ["GET", "HEAD", "POST", "PUT", "DELETE", "OPTIONS", "PATCH"];

/// A destination host as the client named it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Host {
    /// A validated, lower-cased DNS name without a trailing dot.
    Name(String),
    /// An IP literal (dotted IPv4 or bracketed IPv6).
    Ip(IpAddr),
}

impl Host {
    /// The host as logged and as sent in a forwarded `Host` header.
    pub fn display(&self) -> String {
        match self {
            Host::Name(name) => name.clone(),
            Host::Ip(IpAddr::V4(v4)) => v4.to_string(),
            Host::Ip(IpAddr::V6(v6)) => format!("[{v6}]"),
        }
    }
}

/// What the client asked for.
#[derive(Debug)]
pub enum Request {
    /// `CONNECT host:port`.
    Connect { host: Host, port: u16 },
    /// An absolute-form plain HTTP request, ready to forward.
    Plain { host: Host, port: u16, head: Vec<u8> },
}

/// Why a request head is refused before any destination is considered.
#[derive(Debug, PartialEq, Eq)]
pub enum HeadError {
    /// The client closed or failed before sending a full head.
    Closed,
    /// The head did not arrive within the timeout.
    Timeout,
    /// The head is larger than [`MAX_HEAD`].
    TooLarge,
    /// The head is not a well-formed request the proxy accepts.
    Malformed(&'static str),
    /// The method is neither `CONNECT` nor a forwarded plain HTTP method.
    Method,
}

impl HeadError {
    /// The response status and the log reason for this refusal.
    pub fn status(&self) -> (u16, &'static str) {
        match self {
            HeadError::Closed => (400, "client_closed"),
            HeadError::Timeout => (408, "head_timeout"),
            HeadError::TooLarge => (431, "head_too_large"),
            HeadError::Malformed(reason) => (400, reason),
            HeadError::Method => (405, "method_not_allowed"),
        }
    }
}

/// A parsed head: the request, the method as logged, and any bytes the client sent after the head.
pub struct Head {
    pub method: String,
    pub request: Request,
    pub rest: Vec<u8>,
}

/// Reads one head within `timeout`, refusing it once it exceeds [`MAX_HEAD`].
///
/// On failure the method, when one was parsed, is returned for the log.
pub async fn read_head<R: AsyncRead + Unpin>(
    reader: &mut R,
    timeout: Duration,
) -> Result<Head, (HeadError, Option<String>)> {
    let mut buf = Vec::with_capacity(4096);
    let read = async {
        let mut chunk = [0u8; 4096];
        loop {
            let room = MAX_HEAD + 1 - buf.len();
            let n = reader.read(&mut chunk[..room.min(4096)]).await.map_err(|_| HeadError::Closed)?;
            if n == 0 {
                return Err(HeadError::Closed);
            }
            buf.extend_from_slice(&chunk[..n]);
            let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
            let mut req = httparse::Request::new(&mut headers);
            match req.parse(&buf) {
                Ok(httparse::Status::Complete(len)) if len <= MAX_HEAD => return Ok(len),
                Ok(_) if buf.len() > MAX_HEAD => return Err(HeadError::TooLarge),
                Ok(httparse::Status::Complete(_)) => return Err(HeadError::TooLarge),
                Ok(httparse::Status::Partial) => {}
                Err(httparse::Error::TooManyHeaders) => return Err(HeadError::TooLarge),
                Err(_) => return Err(HeadError::Malformed("malformed_head")),
            }
        }
    };
    let len = match tokio::time::timeout(timeout, read).await {
        Ok(Ok(len)) => len,
        Ok(Err(error)) => return Err((error, None)),
        Err(_) => return Err((HeadError::Timeout, None)),
    };
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut req = httparse::Request::new(&mut headers);
    req.parse(&buf[..len]).map_err(|_| (HeadError::Malformed("malformed_head"), None))?;
    let method = req.method.unwrap_or_default().to_owned();
    match parse_request(&req) {
        Ok(request) => Ok(Head { method, request, rest: buf[len..].to_vec() }),
        Err(error) => Err((error, Some(method))),
    }
}

fn parse_request(req: &httparse::Request<'_, '_>) -> Result<Request, HeadError> {
    let method = req.method.unwrap_or_default();
    let target = req.path.unwrap_or_default();
    if req.version != Some(1) && req.version != Some(0) {
        return Err(HeadError::Malformed("bad_version"));
    }
    if method == "CONNECT" {
        let (host, port) = parse_authority(target)?;
        let port = port.ok_or(HeadError::Malformed("missing_port"))?;
        return Ok(Request::Connect { host, port });
    }
    if !PLAIN_METHODS.contains(&method) {
        return Err(HeadError::Method);
    }
    let rest = strip_prefix_ignore_case(target, "http://").ok_or(HeadError::Malformed("not_absolute_http"))?;
    let split = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(split);
    let (host, port) = parse_authority(authority)?;
    let port = port.unwrap_or(80);
    let path = match path.as_bytes().first() {
        None => "/".to_owned(),
        Some(b'?') => format!("/{path}"),
        Some(_) => path.to_owned(),
    };
    let head = forward_head(method, &path, &host, port, req.headers);
    Ok(Request::Plain { host, port, head })
}

/// Builds the upstream head: origin-form target, the absolute URI's `Host`, `Connection: close`, and the client's
/// end-to-end headers only.
fn forward_head(method: &str, path: &str, host: &Host, port: u16, headers: &[httparse::Header<'_>]) -> Vec<u8> {
    let named_hop: Vec<String> = headers
        .iter()
        .filter(|h| h.name.eq_ignore_ascii_case("connection") || h.name.eq_ignore_ascii_case("proxy-connection"))
        .flat_map(|h| {
            String::from_utf8_lossy(h.value).split(',').map(|t| t.trim().to_ascii_lowercase()).collect::<Vec<_>>()
        })
        .filter(|t| !t.is_empty())
        .collect();
    let mut out = Vec::with_capacity(1024);
    out.extend_from_slice(format!("{method} {path} HTTP/1.1\r\n").as_bytes());
    let authority = if port == 80 { host.display() } else { format!("{}:{port}", host.display()) };
    out.extend_from_slice(format!("Host: {authority}\r\n").as_bytes());
    for header in headers {
        let name = header.name.to_ascii_lowercase();
        if is_hop_by_hop(&name) || named_hop.contains(&name) {
            continue;
        }
        push_header(&mut out, header);
    }
    out.extend_from_slice(b"Connection: close\r\n\r\n");
    out
}

/// Appends `header` as `name: value` and its line end.
pub fn push_header(out: &mut Vec<u8>, header: &httparse::Header) {
    out.extend_from_slice(header.name.as_bytes());
    out.extend_from_slice(b": ");
    out.extend_from_slice(header.value);
    out.extend_from_slice(b"\r\n");
}

/// Hop-by-hop headers, `Proxy-*` headers, and `Host` (replaced from the absolute URI). `Transfer-Encoding` stays
/// because the body is relayed unchanged and must keep its framing.
fn is_hop_by_hop(name: &str) -> bool {
    matches!(name, "host" | "connection" | "keep-alive" | "te" | "trailer" | "upgrade") || name.starts_with("proxy-")
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &text[prefix.len()..])
}

/// Parses `host[:port]` where host is a DNS name, a dotted IPv4 literal, or a bracketed IPv6 literal.
pub fn parse_authority(authority: &str) -> Result<(Host, Option<u16>), HeadError> {
    let bad = HeadError::Malformed("bad_authority");
    let (host_text, port_text) = if let Some(rest) = authority.strip_prefix('[') {
        let (inner, after) = rest.split_once(']').ok_or(HeadError::Malformed("bad_authority"))?;
        let ip: std::net::Ipv6Addr = inner.parse().map_err(|_| HeadError::Malformed("bad_authority"))?;
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':').ok_or(HeadError::Malformed("bad_authority"))?),
        };
        return Ok((Host::Ip(IpAddr::V6(ip)), port.map(parse_port).transpose()?));
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    let port = port_text.map(parse_port).transpose()?;
    if let Ok(v4) = host_text.parse::<std::net::Ipv4Addr>() {
        return Ok((Host::Ip(IpAddr::V4(v4)), port));
    }
    let name = validate_name(host_text).ok_or(bad)?;
    Ok((Host::Name(name), port))
}

fn parse_port(text: &str) -> Result<u16, HeadError> {
    let bad = HeadError::Malformed("bad_port");
    if text.is_empty() || text.len() > 5 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad);
    }
    text.parse().map_err(|_| bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorities() {
        let name = |n: &str| Host::Name(n.to_owned());
        assert_eq!(parse_authority("Example.COM.:443"), Ok((name("example.com"), Some(443))));
        assert_eq!(parse_authority("example.com"), Ok((name("example.com"), None)));
        assert_eq!(parse_authority("10.0.0.1:80"), Ok((Host::Ip("10.0.0.1".parse().unwrap()), Some(80))));
        assert_eq!(parse_authority("[::1]:443"), Ok((Host::Ip("::1".parse().unwrap()), Some(443))));
        assert_eq!(parse_authority("[::ffff:127.0.0.1]"), Ok((Host::Ip("::ffff:127.0.0.1".parse().unwrap()), None)));
        for bad in [
            "",
            ":443",
            "user@example.com:443",
            "example.com:",
            "example.com:+443",
            "example.com:70000",
            "[::1",
            "[::1]443",
            "[fe80::1%eth0]:443",
            "::1:443",
            "2130706433:443",
            "0x7f.1:443",
            "127.1:443",
            "exa mple.com:443",
            "-bad.com:443",
        ] {
            assert!(parse_authority(bad).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn forwarded_head_strips_hop_by_hop() {
        let raw = b"GET http://Example.com/a?b=c HTTP/1.1\r\nHost: evil\r\nConnection: keep-alive, X-Hop\r\n\
            Keep-Alive: 5\r\nProxy-Authorization: x\r\nProxy-Connection: keep-alive\r\nTE: trailers\r\n\
            Upgrade: h2c\r\nX-Hop: 1\r\nAccept: */*\r\n\r\n";
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut req = httparse::Request::new(&mut headers);
        req.parse(raw).unwrap();
        let Ok(Request::Plain { head, port, .. }) = parse_request(&req) else { panic!("not plain") };
        assert_eq!(port, 80);
        assert_eq!(
            String::from_utf8(head).unwrap(),
            "GET /a?b=c HTTP/1.1\r\nHost: example.com\r\nAccept: */*\r\nConnection: close\r\n\r\n"
        );
    }
}
