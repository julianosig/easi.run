//! What the admin application and the controller each hold a browser session to, by the same rule: how many may run at
//! once, the address one starts at, and its id.

use std::fmt;
use std::str::FromStr;

/// The most browser sessions at once that the admin's `ADMIN_MAX_SESSIONS` and the controller's
/// `CONTROLLER_MAX_SESSIONS` may allow, and the controller's default.
pub const MAX_SESSIONS: usize = 16;

/// The longest address a session starts at, in bytes.
pub const MAX_URL: usize = 2048;

/// Whether a session may start at `url`: `https://` or `http://` and a host, at most [`MAX_URL`] bytes, without
/// whitespace or a control character. Else what is wrong with it, said of the URL ("must start with ...").
pub fn start_url(url: &str) -> Result<(), String> {
    let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) else {
        return Err("must start with https:// or http://".into());
    };
    // The authority ends where the path, the query or the fragment begins (a backslash begins a path too, as browsers
    // read it); its host follows any user name and precedes any port.
    let authority = rest.split(['/', '\\', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = if host.starts_with('[') { host } else { host.split(':').next().unwrap_or_default() };
    if host.is_empty() {
        return Err("has no host".into());
    }
    if url.len() > MAX_URL {
        return Err(format!("is longer than {MAX_URL} bytes"));
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("contains whitespace or a control character".into());
    }
    Ok(())
}

/// A session's id: twelve lowercase hex characters, its six random bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize), serde(try_from = "String"))]
pub struct SessionId(String);

impl SessionId {
    /// The id of six random bytes.
    pub fn from_bytes(bytes: [u8; 6]) -> SessionId {
        SessionId(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }
}

impl FromStr for SessionId {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        if text.len() == 12 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            Ok(Self(text.to_owned()))
        } else {
            Err("a session id is 12 lowercase hex characters".into())
        }
    }
}

impl TryFrom<String> for SessionId {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        text.parse()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids() {
        for id in ["0123456789ab", "ffffffffffff"] {
            assert!(id.parse::<SessionId>().is_ok());
        }
        for id in ["0123456789AB", "0123456789a", "0123456789abc", "0123456789ag", "../../etc/x", ""] {
            assert!(id.parse::<SessionId>().is_err(), "{id}");
        }
        let id = SessionId::from_bytes([0x01, 0x23, 0x45, 0x67, 0x89, 0xab]);
        assert_eq!(id.to_string(), "0123456789ab");
        assert_eq!(id.to_string().parse(), Ok(id));
    }
}
