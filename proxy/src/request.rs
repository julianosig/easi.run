//! Reading and parsing one bounded request head from a hostile client.

use std::net::IpAddr;
use std::time::Duration;

use egress::destination::validate_name;
use tokio::io::{AsyncRead, AsyncReadExt};

/// The largest request head the proxy reads, terminator included.
pub const MAX_HEAD: usize = 16 * 1024;
const MAX_HEADERS: usize = 100;

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
    /// The method is not `CONNECT`.
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

/// A parsed `CONNECT host:port`, and any bytes the client sent after its head.
pub struct Head {
    pub host: Host,
    pub port: u16,
    pub rest: Vec<u8>,
}

/// Reads one head within `timeout`, refusing it once it exceeds [`MAX_HEAD`].
///
/// On failure the method, when one was parsed, is returned for the log.
pub async fn read_head<R: AsyncRead + Unpin>(
    reader: &mut R,
    timeout: Duration,
) -> Result<Head, (HeadError, Option<String>)> {
    let read = async {
        let mut buf = Vec::with_capacity(4096);
        let mut chunk = [0u8; 4096];
        loop {
            let room = MAX_HEAD + 1 - buf.len();
            let n = reader.read(&mut chunk[..room.min(4096)]).await.map_err(|_| (HeadError::Closed, None))?;
            if n == 0 {
                return Err((HeadError::Closed, None));
            }
            buf.extend_from_slice(&chunk[..n]);
            let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
            let mut req = httparse::Request::new(&mut headers);
            let refused = match req.parse(&buf) {
                Ok(httparse::Status::Complete(len)) if len <= MAX_HEAD => {
                    return match parse_request(&req) {
                        Ok((host, port)) => Ok(Head { host, port, rest: buf[len..].to_vec() }),
                        Err(error) => Err((error, req.method.map(str::to_owned))),
                    };
                }
                Ok(_) if buf.len() > MAX_HEAD => HeadError::TooLarge,
                Ok(httparse::Status::Complete(_)) => HeadError::TooLarge,
                Ok(httparse::Status::Partial) => continue,
                Err(httparse::Error::TooManyHeaders) => HeadError::TooLarge,
                Err(_) => HeadError::Malformed("malformed_head"),
            };
            return Err((refused, None));
        }
    };
    tokio::time::timeout(timeout, read).await.unwrap_or(Err((HeadError::Timeout, None)))
}

fn parse_request(req: &httparse::Request<'_, '_>) -> Result<(Host, u16), HeadError> {
    if req.version != Some(1) && req.version != Some(0) {
        return Err(HeadError::Malformed("bad_version"));
    }
    if req.method != Some("CONNECT") {
        return Err(HeadError::Method);
    }
    let (host, port) = parse_authority(req.path.unwrap_or_default())?;
    Ok((host, port.ok_or(HeadError::Malformed("missing_port"))?))
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
}
