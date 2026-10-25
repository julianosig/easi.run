//! The upstream proxy a session's traffic leaves through, when the deployment configures one: parsed from the admin's
//! configuration file and checked again, by the same rules, by the controller and by the session's proxy. Its
//! credentials never appear in `Debug` output or in an error. [`destination`] holds the destination rules the two
//! share the same way.

use std::fmt;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

pub mod destination;

/// Largest user name or password, in bytes (SOCKS5's own bound).
const MAX_CREDENTIAL: usize = 255;
/// Written in a configured user name, replaced by the session's id: providers that keep one exit address per user
/// name session then keep one per browser session.
const SESSION_PLACEHOLDER: &str = "{session}";

/// How the session's proxy speaks to the upstream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    /// HTTP `CONNECT` over plain TCP (credentials, if any, travel in the clear to the upstream).
    Http,
    /// HTTP `CONNECT` over TLS, the upstream's certificate verified against the public roots.
    Https,
    /// SOCKS5 over plain TCP (RFC 1928, with RFC 1929 user name and password).
    Socks5,
}

/// One upstream proxy.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Upstream {
    pub scheme: Scheme,
    /// A lower-case DNS name or an IP literal (without brackets).
    pub host: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

impl fmt::Debug for Upstream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let credentials = if self.username.is_some() { "present" } else { "none" };
        write!(f, "Upstream({:?} {}:{}, credentials {credentials})", self.scheme, self.host, self.port)
    }
}

impl Upstream {
    /// Parses `scheme://[user[:password]@]host:port`, the user name and password percent-encoded. Errors say what is
    /// wrong, never what was written.
    pub fn parse_url(text: &str) -> Result<Upstream, &'static str> {
        let (scheme, rest) = text.split_once("://").ok_or("not a URL")?;
        let scheme = match scheme {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            "socks5" => Scheme::Socks5,
            _ => return Err("the scheme is not http, https, or socks5"),
        };
        let (userinfo, authority) = match rest.rsplit_once('@') {
            Some((userinfo, authority)) => (Some(userinfo), authority),
            None => (None, rest),
        };
        let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
            let (host, port) = bracketed.split_once("]:").ok_or("the address needs a port")?;
            (host, port)
        } else {
            authority.rsplit_once(':').ok_or("the address needs a port")?
        };
        let port = port.parse::<u16>().ok().filter(|&p| p > 0).ok_or("the port is not 1 to 65535")?;
        let (username, password) = match userinfo {
            None => (None, None),
            Some(userinfo) => {
                let (user, password) = match userinfo.split_once(':') {
                    Some((user, password)) => (user, Some(password)),
                    None => (userinfo, None),
                };
                (Some(percent_decode(user)?), password.map(percent_decode).transpose()?)
            }
        };
        let upstream = Upstream { scheme, host: host.to_ascii_lowercase(), port, username, password };
        upstream.validate()?;
        Ok(upstream)
    }

    /// The rules every holder applies.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.port == 0 {
            return Err("the port is not 1 to 65535");
        }
        // A name as the proxy admits a destination's, already in that form: lower-case, without a trailing dot.
        let name = || destination::validate_name(&self.host).as_deref() == Some(self.host.as_str());
        if self.host.parse::<IpAddr>().is_err() && !name() {
            return Err("the host is neither a DNS name nor an IP address");
        }
        let credential = |c: &str| c.len() <= MAX_CREDENTIAL && !c.chars().any(char::is_control);
        if !self.username.as_deref().is_none_or(credential) || !self.password.as_deref().is_none_or(credential) {
            return Err("a credential is longer than 255 bytes or holds a control character");
        }
        if self.password.is_some() && self.username.is_none() {
            return Err("a password needs a user name");
        }
        match self.scheme {
            // RFC 7617: the user name of Basic authentication cannot hold a colon.
            Scheme::Http | Scheme::Https if self.username.as_deref().is_some_and(|u| u.contains(':')) => {
                Err("an HTTP proxy's user name cannot hold a colon")
            }
            // RFC 1929: both are one to 255 bytes.
            Scheme::Socks5
                if self.username.is_some()
                    && (self.username.as_deref() == Some("") || self.password.as_deref().is_none_or(str::is_empty)) =>
            {
                Err("a SOCKS5 proxy needs a non-empty user name and password together")
            }
            _ => Ok(()),
        }
    }

    /// This upstream for one session: the placeholder in the user name becomes the session's id, and the result is
    /// checked again (its length may have grown).
    pub fn for_session(&self, session: &str) -> Result<Upstream, &'static str> {
        let mut upstream = self.clone();
        if let Some(user) = upstream.username.as_mut() {
            *user = user.replace(SESSION_PLACEHOLDER, session);
        }
        upstream.validate()?;
        Ok(upstream)
    }

    /// Whether its user name holds the session placeholder: each session then leaves through an upstream of its own.
    pub fn per_session(&self) -> bool {
        self.username.as_deref().is_some_and(|user| user.contains(SESSION_PLACEHOLDER))
    }

    /// The `Proxy-Authorization` credentials of an HTTP upstream, `user:password`, if it has any.
    pub fn basic_credentials(&self) -> Option<String> {
        let user = self.username.as_deref()?;
        Some(format!("{user}:{}", self.password.as_deref().unwrap_or_default()))
    }
}

fn percent_decode(text: &str) -> Result<String, &'static str> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
            out.push(hex.and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or("a credential has a bad escape")?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "a credential is not UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_parse_to_their_parts() {
        let u = Upstream::parse_url("http://user-{session}:p%40ss@Proxy.Example.net:8080").unwrap();
        assert_eq!(u.scheme, Scheme::Http);
        assert_eq!((u.host.as_str(), u.port), ("proxy.example.net", 8080));
        assert_eq!((u.username.as_deref(), u.password.as_deref()), (Some("user-{session}"), Some("p@ss")));
        assert_eq!(u.basic_credentials().as_deref(), Some("user-{session}:p@ss"));
        let v6 = Upstream::parse_url("socks5://[2001:db8::1]:1080").unwrap();
        assert_eq!((v6.host.as_str(), v6.port, v6.username), ("2001:db8::1", 1080, None));
        assert_eq!(Upstream::parse_url("https://10.0.0.5:3128").unwrap().scheme, Scheme::Https);
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in [
            "proxy.example.net:8080",
            "ftp://proxy.example.net:21",
            "http://proxy.example.net",
            "http://proxy.example.net:0",
            "http://proxy.example.net:70000",
            "http://proxy.example.net:8080/path",
            "http://2130706433:8080",
            "http://proxy.example.net.:8080",
            "http://-proxy.net:8080",
            "http://us%3Aer:pw@proxy.net:8080",
            "http://user:p%0Aw@proxy.net:8080",
            "http://user:p%zzw@proxy.net:8080",
            "socks5://user@proxy.net:1080",
            "socks5://:pw@proxy.net:1080",
        ] {
            assert!(Upstream::parse_url(bad).is_err(), "{bad}");
        }
        let long = format!("http://{}:pw@proxy.net:8080", "u".repeat(MAX_CREDENTIAL + 1));
        assert!(Upstream::parse_url(&long).is_err());
    }

    #[test]
    fn a_session_id_fills_the_placeholder_and_is_checked_again() {
        let u = Upstream::parse_url("http://user-{session}:pw@proxy.net:8080").unwrap();
        assert_eq!(u.for_session("0123456789ab").unwrap().username.as_deref(), Some("user-0123456789ab"));
        assert!(u.per_session() && !u.for_session("0123456789ab").unwrap().per_session());
        assert!(!Upstream::parse_url("http://user:pw@proxy.net:8080").unwrap().per_session());
        let near = format!("http://{}{{session}}:pw@proxy.net:8080", "u".repeat(MAX_CREDENTIAL - 9));
        assert!(Upstream::parse_url(&near).unwrap().for_session("0123456789ab").is_err());
    }

    #[test]
    fn debug_output_holds_no_credential() {
        let u = Upstream::parse_url("http://alice:secret@proxy.net:8080").unwrap();
        let shown = format!("{u:?}");
        assert!(!shown.contains("alice") && !shown.contains("secret"), "{shown}");
    }
}
