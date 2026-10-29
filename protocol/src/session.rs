//! What the admin application and the controller each hold a browser session to, by the same rule: how many may run at
//! once, and the address one starts at.

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
