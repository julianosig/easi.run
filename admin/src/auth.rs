//! The one person who administers this installation.
//!
//! While no password is set, the admin holds one enrollment token, which setting the password requires: single use,
//! valid for 15 minutes, and replaced by a new one once it expires, so a valid one is always there. It is never logged:
//! the admin writes it, with its expiry, to the private setup file of its state directory, and `admin setup-link`
//! prints the link that carries it to the admin's first screen; the file goes once a password is set. `admin init` sets
//! the password instead, from its stdin, before or while the admin runs; neither replaces a password that is set.
//! Passwords are hashed with Argon2id. A login (setting the password opens one too) creates a session: a `__Host-`
//! cookie that is `Secure`, `HttpOnly`, and `SameSite=Strict`, with an idle and an absolute expiry, and a CSRF token
//! every state-changing request must echo. Every state-changing request and every WebSocket upgrade must also come
//! from the exact configured origin.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::http::{HeaderMap, Method, header};
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const COOKIE: &str = "__Host-display";
pub const CSRF_HEADER: &str = "x-csrf-token";
const ENROLLMENT_LIFETIME: Duration = Duration::from_secs(15 * 60);
/// The setup file, in the state directory: the current enrollment token and its expiry (see [`Setup`]).
const SETUP: &str = "setup";
pub const IDLE_EXPIRY: Duration = Duration::from_secs(30 * 60);
pub const ABSOLUTE_EXPIRY: Duration = Duration::from_secs(12 * 60 * 60);
const MIN_PASSWORD: usize = 12;
const MAX_PASSWORD: usize = 1024;
/// Failed logins in a row before each further attempt waits.
const FREE_FAILURES: u32 = 5;
/// Password checks allowed to run at once; more are refused, so parallel guesses neither outrun the failure backoff
/// nor occupy every blocking thread.
const MAX_VERIFYING: u32 = 2;

/// The opaque id of a login session (the SHA-256 of its cookie value).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoginKey([u8; 32]);

pub struct Login {
    pub csrf: String,
    created: Instant,
    last_seen: Instant,
    /// A hard end (a demo visitor's deadline): the login is dead from then on, whatever else happens.
    deadline: Option<Instant>,
}

pub struct Auth {
    password_file: PathBuf,
    setup_file: PathBuf,
    hash: Option<String>,
    /// Whether the password file is read again while no password is set (`admin init` may set one): outside the demo,
    /// until it holds a password or one it cannot use.
    rereads: bool,
    enrollment: Option<(String, Instant)>,
    sessions: HashMap<LoginKey, Login>,
    failures: u32,
    locked_until: Option<Instant>,
    /// Password checks running now: each costs an Argon2id hash on a blocking thread.
    verifying: u32,
}

/// What `admin init` found (see [`Auth::initialize`]).
#[derive(Debug, PartialEq, Eq)]
pub enum Initialized {
    /// The password is the one given: set now, or set to it already.
    Set,
    /// Another password was set already: it is left as it was.
    Other,
}

/// What `admin setup-link` found (see [`Auth::setup_token`]).
#[derive(Debug, PartialEq, Eq)]
pub enum SetupLink {
    /// The running admin's enrollment token, still valid.
    Token(String),
    /// A password is set: nothing is left to set up.
    Set,
}

/// The setup file's contents: the enrollment token and when it expires, in Unix seconds, so that `admin setup-link`,
/// another process, can tell an expired one.
#[derive(Serialize, Deserialize)]
struct Setup {
    token: String,
    expires: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Unauthenticated,
    Forbidden,
    BadRequest(&'static str),
    TooManyAttempts,
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// The login a cookie value names.
pub fn key_of(cookie_value: &str) -> LoginKey {
    LoginKey(Sha256::digest(cookie_value.as_bytes()).into())
}

fn equal(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

impl Auth {
    /// Loads the password hash, or starts enrollment (see [`Auth::renew`]). The public demo has no password and never
    /// enrolls (`enrollable` false): its visitors get in without one. Only a password file that is not there opens
    /// enrollment: one that cannot be read, or holds no hash Argon2 can verify, stops the start, since enrolling would
    /// let whoever holds the setup link replace the administrator's password, and a hash it cannot verify would fail
    /// every login.
    pub fn load(state_dir: &Path, enrollable: bool) -> Result<Auth, String> {
        let password_file = state_dir.join("password");
        let hash = Auth::read_hash(&password_file)?;
        let mut auth = Auth {
            password_file,
            setup_file: state_dir.join(SETUP),
            hash,
            rereads: enrollable,
            enrollment: None,
            sessions: HashMap::new(),
            failures: 0,
            locked_until: None,
            verifying: 0,
        };
        auth.renew();
        if auth.enrollment.is_some() {
            tracing::warn!("no password is set; `admin setup-link` prints the link to the screen that sets it");
        }
        Ok(auth)
    }

    /// The hash in a password file: none when the file is not there, an error when it cannot be read or holds no hash
    /// Argon2 can verify.
    pub fn read_hash(password_file: &Path) -> Result<Option<String>, String> {
        match std::fs::read_to_string(password_file) {
            Ok(text) => {
                let hash = text.trim().to_owned();
                // Checked by verifying a password it cannot hold (the shortest allowed is 12 characters): only a whole
                // hash Argon2 can verify answers that it does not match, rather than failing to read it.
                let verifiable = PasswordHash::new(&hash).is_ok_and(|parsed| {
                    parsed.salt.is_some()
                        && parsed.hash.is_some()
                        && matches!(
                            argon2::Argon2::default().verify_password(b"", &parsed),
                            Err(argon2::password_hash::Error::Password)
                        )
                });
                if !verifiable {
                    return Err(format!("{} holds no password hash; restore it or remove it", password_file.display()));
                }
                Ok(Some(hash))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{} cannot be read: {e}", password_file.display())),
        }
    }

    pub fn enrolled(&self) -> bool {
        self.hash.is_some()
    }

    /// The password file to read again (with [`Auth::read_hash`], off the lock) while no password is set, enrollment
    /// open, expired or lost to it: `admin init` may have set one since the start.
    pub fn pending(&self) -> Option<PathBuf> {
        (self.rereads && self.hash.is_none()).then(|| self.password_file.clone())
    }

    /// Takes what [`Auth::read_hash`] found in the [`Auth::pending`] file. A password set meanwhile closes enrollment;
    /// so does a file that holds none, which enrolling could not replace, and which is not read again.
    pub fn adopt(&mut self, found: Result<Option<String>, String>) {
        if self.pending().is_none() {
            return;
        }
        match found {
            Ok(None) => {}
            Ok(Some(hash)) => {
                self.enrollment = None;
                self.hash = Some(hash);
                self.close_setup();
                tracing::info!("a password was set (admin init); enrollment closed");
            }
            Err(why) => {
                self.enrollment = None;
                self.rereads = false;
                self.close_setup();
                tracing::warn!(%why, "enrollment closed");
            }
        }
    }

    /// Keeps a valid enrollment token while no password is set (outside the demo): a new one once the last has expired
    /// (or never came), written to the setup file over the last one's. The token is good only once that write is done,
    /// so the link `admin setup-link` prints always holds it; a write that fails leaves enrollment closed, and the next
    /// call tries again. Written under the lock, which nothing waits on for long while no password is set: there is no
    /// login yet whose requests it could stall.
    pub fn renew(&mut self) {
        let valid = self.enrollment.as_ref().is_some_and(|(_, issued)| issued.elapsed() <= ENROLLMENT_LIFETIME);
        if valid || self.pending().is_none() {
            return;
        }
        self.enrollment = None;
        let token = random_token();
        let expires = SystemTime::now() + ENROLLMENT_LIFETIME;
        let setup = Setup {
            token: token.clone(),
            expires: expires.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
        };
        let json = serde_json::to_vec(&setup).expect("the setup serializes");
        match crate::private::replace(&self.setup_file, &json) {
            Ok(()) => self.enrollment = Some((token, Instant::now())),
            Err(why) => tracing::warn!(%why, "the setup link could not be written; trying again shortly"),
        }
    }

    /// Removes the setup file once no token in it can set the password.
    fn close_setup(&self) {
        match std::fs::remove_file(&self.setup_file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(why = %e, "the setup file could not be removed"),
        }
    }

    /// `admin setup-link`: the running admin's enrollment token, from the setup file of `state_dir`, unless a password
    /// is set. One past its expiry, or no file yet, is waited for up to `wait`: the running admin replaces an expired
    /// token within seconds (see [`Auth::renew`]). A password file or setup file that cannot be read is an error.
    pub fn setup_token(state_dir: &Path, wait: Duration) -> Result<SetupLink, String> {
        let deadline = Instant::now() + wait;
        loop {
            if Auth::read_hash(&state_dir.join("password"))?.is_some() {
                return Ok(SetupLink::Set);
            }
            if let Some(token) = read_setup(&state_dir.join(SETUP))? {
                return Ok(SetupLink::Token(token));
            }
            if Instant::now() >= deadline {
                return Err(
                    "no setup link is ready; is the admin running? Its log says why: docker compose logs admin".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    // Enrollment and login each take three steps, so the slow Argon2id work runs outside the lock (callers run it on a
    // blocking thread): a quick check under the lock, the hash or verification, and a quick finish under the lock that
    // checks the state again.

    /// Enrollment, step 1: the token is right and still valid, and the password's length is allowed.
    pub fn check_enrollment(&mut self, token: &str, password: &str) -> Result<(), Refusal> {
        let Some((expected, issued)) = &self.enrollment else { return Err(Refusal::Forbidden) };
        if issued.elapsed() > ENROLLMENT_LIFETIME {
            self.enrollment = None;
            return Err(Refusal::Forbidden);
        }
        if !equal(token, expected) {
            return Err(Refusal::Forbidden);
        }
        if !(MIN_PASSWORD..=MAX_PASSWORD).contains(&password.chars().count()) {
            return Err(Refusal::BadRequest("the password needs 12 to 1024 characters"));
        }
        Ok(())
    }

    /// Enrollment, step 2 (slow, no lock): the password's Argon2id hash.
    pub fn hash_password(password: &str) -> Result<String, Refusal> {
        let salt = SaltString::encode_b64(&rand::random::<[u8; 16]>()).expect("a 16-byte salt encodes");
        argon2::Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|_| Refusal::BadRequest("the password could not be hashed"))
    }

    /// Enrollment, step 3: stores the hash if the same token still holds enrollment open, and no password file is
    /// there (`admin init` may have written one meanwhile; it is never replaced), then opens a login, as a login does,
    /// and returns its cookie value and CSRF token. The token is single-use whatever happens next.
    pub fn complete_enrollment(&mut self, token: &str, hash: String) -> Result<(String, String), Refusal> {
        match &self.enrollment {
            Some((expected, issued)) if issued.elapsed() <= ENROLLMENT_LIFETIME && equal(token, expected) => {}
            _ => return Err(Refusal::Forbidden),
        }
        self.enrollment = None;
        match crate::private::create(&self.password_file, hash.as_bytes()) {
            Ok(true) => {}
            Ok(false) => {
                // `admin init` set one meanwhile: taken now rather than at the next reread, so the page, asking at
                // once, hears that a password is set and offers the login.
                self.adopt(Auth::read_hash(&self.password_file));
                return Err(Refusal::Forbidden);
            }
            Err(_) => return Err(Refusal::Forbidden),
        }
        self.hash = Some(hash);
        self.close_setup();
        tracing::info!("password set; enrollment closed");
        Ok(self.open_login())
    }

    /// `admin init`: sets the password from `password` unless one is set already, exactly as enrollment stores it, in
    /// the password file of `state_dir`, which it never replaces; a password set already is checked against
    /// `password`. A password file that cannot be read or holds no hash is an error, as at the start.
    pub fn initialize(state_dir: &Path, password: &str) -> Result<Initialized, String> {
        if !(MIN_PASSWORD..=MAX_PASSWORD).contains(&password.chars().count()) {
            return Err(format!("the password needs {MIN_PASSWORD} to {MAX_PASSWORD} characters"));
        }
        let password_file = state_dir.join("password");
        let set = |hash: Option<String>| match hash {
            Some(hash) if Auth::verify_password(&hash, password) => Ok(Initialized::Set),
            Some(_) => Ok(Initialized::Other),
            None => Err(format!("{} is not there", password_file.display())),
        };
        if let Some(hash) = Auth::read_hash(&password_file)? {
            return set(Some(hash));
        }
        let hash = Auth::hash_password(password).map_err(|_| "the password could not be hashed".to_owned())?;
        if crate::private::create(&password_file, hash.as_bytes())? {
            // The setup file's token can set nothing now (a password is never replaced); a stopped admin's would
            // otherwise stay until its next start. Not there, or not removed, the password is set all the same.
            let _ = std::fs::remove_file(state_dir.join(SETUP));
            Ok(Initialized::Set)
        } else {
            // Another one was set meanwhile.
            set(Auth::read_hash(&password_file)?)
        }
    }

    /// Login, step 1: not locked out and not too many checks running; returns the stored hash to verify against.
    pub fn begin_login(&mut self) -> Result<String, Refusal> {
        if self.locked_until.is_some_and(|t| Instant::now() < t) || self.verifying >= MAX_VERIFYING {
            return Err(Refusal::TooManyAttempts);
        }
        let hash = self.hash.clone().ok_or(Refusal::Forbidden)?;
        self.verifying += 1;
        Ok(hash)
    }

    /// Login, step 2 (slow, no lock): whether the password matches the hash.
    pub fn verify_password(hash: &str, password: &str) -> bool {
        password.len() <= MAX_PASSWORD * 4
            && PasswordHash::new(hash)
                .is_ok_and(|parsed| argon2::Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
    }

    /// Login, step 3: counts a failure toward the backoff, or opens a session and returns the cookie value and the
    /// CSRF token.
    pub fn finish_login(&mut self, matched: bool) -> Result<(String, String), Refusal> {
        self.verifying = self.verifying.saturating_sub(1);
        if !matched {
            self.failures += 1;
            if self.failures >= FREE_FAILURES {
                let wait = Duration::from_secs(2u64.pow((self.failures + 1 - FREE_FAILURES).min(8)));
                self.locked_until = Some(Instant::now() + wait);
            }
            return Err(Refusal::Unauthenticated);
        }
        self.failures = 0;
        self.locked_until = None;
        Ok(self.open_login())
    }

    /// Opens a session without a password (a demo visitor holding the slot) and returns the cookie value and the
    /// CSRF token.
    pub fn open_login(&mut self) -> (String, String) {
        let cookie = random_token();
        let csrf = random_token();
        let now = Instant::now();
        self.sessions
            .insert(key_of(&cookie), Login { csrf: csrf.clone(), created: now, last_seen: now, deadline: None });
        (cookie, csrf)
    }

    /// All three enrollment steps at once, for tests.
    #[cfg(test)]
    pub fn enroll(&mut self, token: &str, password: &str) -> Result<(String, String), Refusal> {
        self.check_enrollment(token, password)?;
        let hash = Auth::hash_password(password)?;
        self.complete_enrollment(token, hash)
    }

    /// All three login steps at once, for tests.
    #[cfg(test)]
    pub fn login(&mut self, password: &str) -> Result<(String, String), Refusal> {
        let hash = self.begin_login()?;
        self.finish_login(Auth::verify_password(&hash, password))
    }

    /// Ends a login at `deadline` at the latest.
    pub fn set_deadline(&mut self, key: LoginKey, deadline: Instant) {
        if let Some(login) = self.sessions.get_mut(&key) {
            login.deadline = Some(deadline);
        }
    }

    pub fn logout(&mut self, key: LoginKey) {
        self.sessions.remove(&key);
    }

    /// Authenticates a request: a live session cookie and, unless the method is safe, the exact origin and the
    /// session's CSRF token. Touches the session.
    pub fn check(&mut self, method: &Method, headers: &HeaderMap, origin: &str) -> Result<(LoginKey, String), Refusal> {
        let key = cookie(headers).map(|v| key_of(&v)).ok_or(Refusal::Unauthenticated)?;
        let login = self.live_mut(&key)?;
        let safe = matches!(*method, Method::GET | Method::HEAD);
        if !safe {
            if !origin_matches(headers, origin) {
                return Err(Refusal::Forbidden);
            }
            let token = headers.get(CSRF_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
            if !equal(token, &login.csrf) {
                return Err(Refusal::Forbidden);
            }
        }
        login.last_seen = Instant::now();
        Ok((key, login.csrf.clone()))
    }

    /// A WebSocket upgrade: the exact origin and a live session.
    pub fn check_upgrade(&mut self, headers: &HeaderMap, origin: &str) -> Result<LoginKey, Refusal> {
        if !origin_matches(headers, origin) {
            return Err(Refusal::Forbidden);
        }
        let key = cookie(headers).map(|v| key_of(&v)).ok_or(Refusal::Unauthenticated)?;
        let login = self.live_mut(&key)?;
        login.last_seen = Instant::now();
        Ok(key)
    }

    /// A live login, to touch. An expired one is refused but left for the sweep (`expire`), which alone removes it, so
    /// its streams are always closed with it.
    fn live_mut(&mut self, key: &LoginKey) -> Result<&mut Login, Refusal> {
        self.sessions.get_mut(key).filter(|l| live(l, Instant::now())).ok_or(Refusal::Unauthenticated)
    }

    /// Whether the login is still live (not logged out or expired).
    pub fn is_live(&self, key: &LoginKey) -> bool {
        self.sessions.get(key).is_some_and(|l| live(l, Instant::now()))
    }

    /// Drops expired sessions and returns them, so their streams can be closed.
    pub fn expire(&mut self) -> Vec<LoginKey> {
        let now = Instant::now();
        let expired: Vec<LoginKey> = self.sessions.iter().filter(|(_, l)| !live(l, now)).map(|(k, _)| *k).collect();
        for key in &expired {
            self.sessions.remove(key);
        }
        expired
    }
}

/// The token in a setup file, unless the file is not there or the token has expired.
fn read_setup(file: &Path) -> Result<Option<String>, String> {
    let text = match std::fs::read(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{} cannot be read: {e}", file.display())),
    };
    let setup: Setup =
        serde_json::from_slice(&text).map_err(|_| format!("{} holds no setup link; remove it", file.display()))?;
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Ok((setup.expires > now).then_some(setup.token))
}

fn live(login: &Login, now: Instant) -> bool {
    now.duration_since(login.last_seen) < IDLE_EXPIRY
        && now.duration_since(login.created) < ABSOLUTE_EXPIRY
        && login.deadline.is_none_or(|d| now < d)
}

/// The request comes from exactly `origin`.
pub fn origin_matches(headers: &HeaderMap, origin: &str) -> bool {
    headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) == Some(origin)
}

fn cookie(headers: &HeaderMap) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == COOKIE && !value.is_empty()).then(|| value.to_owned())
    })
}

pub fn set_cookie(value: &str) -> String {
    format!("{COOKIE}={value}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={}", ABSOLUTE_EXPIRY.as_secs())
}

pub fn clear_cookie() -> String {
    format!("{COOKIE}=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "https://localhost:9443";

    fn headers(cookie: Option<&str>, origin: Option<&str>, csrf: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(c) = cookie {
            h.insert(header::COOKIE, format!("other=1; {COOKIE}={c}").parse().unwrap());
        }
        if let Some(o) = origin {
            h.insert(header::ORIGIN, o.parse().unwrap());
        }
        if let Some(t) = csrf {
            h.insert(CSRF_HEADER, t.parse().unwrap());
        }
        h
    }

    #[test]
    fn parallel_password_checks_are_bounded() {
        let dir = crate::private::test_dir("auth");
        let mut auth = Auth::load(&dir, true).unwrap();
        let token = auth.enrollment.as_ref().unwrap().0.clone();
        auth.enroll(&token, "correct horse battery").unwrap();
        let hash = auth.begin_login().unwrap();
        auth.begin_login().unwrap();
        assert_eq!(auth.begin_login(), Err(Refusal::TooManyAttempts), "a third check at once is refused");
        assert!(auth.finish_login(Auth::verify_password(&hash, "correct horse battery")).is_ok());
        assert!(auth.begin_login().is_ok(), "a finished check frees its place");
    }

    #[test]
    fn enrollment_is_single_use_and_needs_the_token() {
        let dir = crate::private::test_dir("auth");
        let mut auth = Auth::load(&dir, true).unwrap();
        let token = auth.enrollment.as_ref().unwrap().0.clone();
        assert_eq!(auth.enroll("wrong", "correct horse battery"), Err(Refusal::Forbidden));
        assert!(matches!(auth.enroll(&token, "short"), Err(Refusal::BadRequest(_))));
        auth.enroll(&token, "correct horse battery").unwrap();
        assert_eq!(auth.enroll(&token, "another long password"), Err(Refusal::Forbidden));
        // The hash persists and enrollment stays closed after a restart.
        let again = Auth::load(&dir, true).unwrap();
        assert!(again.enrolled() && again.enrollment.is_none());
    }

    #[test]
    fn requests_need_cookie_origin_and_csrf() {
        let dir = crate::private::test_dir("auth");
        let mut auth = Auth::load(&dir, true).unwrap();
        let token = auth.enrollment.as_ref().unwrap().0.clone();
        auth.enroll(&token, "correct horse battery").unwrap();
        assert_eq!(auth.login("nope nope nope"), Err(Refusal::Unauthenticated));
        let (cookie, csrf) = auth.login("correct horse battery").unwrap();
        assert!(auth.check(&Method::GET, &headers(Some(&cookie), None, None), ORIGIN).is_ok());
        assert_eq!(auth.check(&Method::GET, &headers(None, None, None), ORIGIN).unwrap_err(), Refusal::Unauthenticated);
        let mut post = |h| auth_check_post(&mut auth, h);
        assert_eq!(post(headers(Some(&cookie), Some(ORIGIN), None)), Err(Refusal::Forbidden));
        assert_eq!(post(headers(Some(&cookie), Some("https://evil.example"), Some(&csrf))), Err(Refusal::Forbidden));
        assert_eq!(post(headers(Some(&cookie), None, Some(&csrf))), Err(Refusal::Forbidden));
        assert!(post(headers(Some(&cookie), Some(ORIGIN), Some(&csrf))).is_ok());
        assert_eq!(
            auth.check_upgrade(&headers(Some(&cookie), Some("https://evil.example"), None), ORIGIN),
            Err(Refusal::Forbidden)
        );
        let key = auth.check_upgrade(&headers(Some(&cookie), Some(ORIGIN), None), ORIGIN).unwrap();
        auth.logout(key);
        assert_eq!(
            auth.check(&Method::GET, &headers(Some(&cookie), None, None), ORIGIN).unwrap_err(),
            Refusal::Unauthenticated
        );
    }

    fn auth_check_post(auth: &mut Auth, h: HeaderMap) -> Result<(), Refusal> {
        auth.check(&Method::POST, &h, ORIGIN).map(|_| ())
    }

    #[test]
    fn idle_and_absolute_expiry_end_sessions() {
        let dir = crate::private::test_dir("auth");
        let mut auth = Auth::load(&dir, true).unwrap();
        let token = auth.enrollment.as_ref().unwrap().0.clone();
        auth.enroll(&token, "correct horse battery").unwrap();
        let (cookie, _) = auth.login("correct horse battery").unwrap();
        let key = key_of(&cookie);
        auth.sessions.get_mut(&key).unwrap().last_seen -= IDLE_EXPIRY;
        let get = headers(Some(&cookie), Some(ORIGIN), None);
        assert_eq!(auth.check(&Method::GET, &get, ORIGIN).unwrap_err(), Refusal::Unauthenticated);
        assert_eq!(auth.check_upgrade(&get, ORIGIN).unwrap_err(), Refusal::Unauthenticated);
        assert_eq!(auth.expire(), vec![key], "a refusal leaves it for the sweep, which closes its streams");
        let (cookie, _) = auth.login("correct horse battery").unwrap();
        let key = key_of(&cookie);
        auth.sessions.get_mut(&key).unwrap().created -= ABSOLUTE_EXPIRY;
        assert!(!auth.is_live(&key));
    }

    #[test]
    fn repeated_failures_slow_logins_down() {
        let dir = crate::private::test_dir("auth");
        let mut auth = Auth::load(&dir, true).unwrap();
        let token = auth.enrollment.as_ref().unwrap().0.clone();
        auth.enroll(&token, "correct horse battery").unwrap();
        for _ in 0..FREE_FAILURES {
            assert_eq!(auth.login("wrong password!!"), Err(Refusal::Unauthenticated));
        }
        assert_eq!(auth.login("correct horse battery"), Err(Refusal::TooManyAttempts));
    }

    #[test]
    fn a_login_dies_at_its_deadline() {
        let dir = crate::private::test_dir("auth");
        let mut auth = Auth::load(&dir, true).unwrap();
        let token = auth.enrollment.as_ref().unwrap().0.clone();
        auth.enroll(&token, "correct horse battery").unwrap();
        let (cookie, _) = auth.login("correct horse battery").unwrap();
        let key = key_of(&cookie);
        auth.set_deadline(key, Instant::now() + Duration::from_secs(3600));
        assert!(auth.is_live(&key));
        auth.set_deadline(key, Instant::now());
        assert!(!auth.is_live(&key));
        assert_eq!(auth.expire(), vec![key], "the sweep ends it even if nothing else does");
    }

    #[test]
    fn only_a_password_file_that_is_not_there_opens_enrollment() {
        let dir = crate::private::test_dir("auth");
        let mut auth = Auth::load(&dir, true).unwrap();
        let token = auth.enrollment.as_ref().unwrap().0.clone();
        auth.enroll(&token, "correct horse battery").unwrap();
        assert!(Auth::load(&dir, true).unwrap().enrolled(), "the hash it wrote loads");
        let file = dir.join("password");
        let truncated = [
            &b"$argon2id"[..],
            b"$argon2id$v=19$m=19456,t=2,p=1",
            b"$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ",
            b"$argon2id$v=99$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA",
            b"$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaGhhc2hoYXNoaGFzaA",
        ];
        for corrupted in [&b""[..], b"\n", b"not a hash\n", b"$argon2id$v=19$m=19456,t=2,p=1$\xff\n", b"$pbkdf2$x$y"]
            .into_iter()
            .chain(truncated)
        {
            std::fs::write(&file, corrupted).unwrap();
            let why = Auth::load(&dir, true).err().unwrap_or_else(|| panic!("{corrupted:?} opened the start"));
            assert!(why.contains("password"), "{why}");
        }
        // Unreadable: a directory where the file should be (a mode would not stop a test run as root).
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(Auth::load(&dir, true).is_err());
    }

    #[test]
    fn init_sets_the_password_once_and_enrollment_never_replaces_it() {
        let dir = crate::private::test_dir("auth");
        let mut running = Auth::load(&dir, true).unwrap();
        let token = running.enrollment.as_ref().unwrap().0.clone();
        assert!(Auth::initialize(&dir, "short").is_err(), "12 characters at least");
        assert_eq!(Auth::initialize(&dir, "correct horse battery"), Ok(Initialized::Set));
        assert_eq!(Auth::initialize(&dir, "correct horse battery"), Ok(Initialized::Set), "the same one again");
        assert_eq!(Auth::initialize(&dir, "another long password"), Ok(Initialized::Other));
        let set = std::fs::read_to_string(dir.join("password")).unwrap();
        // An admin that started before it still has enrollment open: completing it leaves the password alone, and
        // takes the one set at once.
        assert_eq!(running.enroll(&token, "an enrolled password"), Err(Refusal::Forbidden));
        assert_eq!(std::fs::read_to_string(dir.join("password")).unwrap(), set);
        assert!(running.enrolled() && running.login("correct horse battery").is_ok());
        let mut auth = Auth::load(&dir, true).unwrap();
        assert!(auth.enrollment.is_none());
        assert!(auth.login("correct horse battery").is_ok(), "the password init set logs in");
        std::fs::write(dir.join("password"), "not a hash").unwrap();
        assert!(Auth::initialize(&dir, "correct horse battery").unwrap_err().contains("password"), "corrupt: an error");
        assert_eq!(std::fs::read_to_string(dir.join("password")).unwrap(), "not a hash", "left as it was");
    }

    #[test]
    fn a_running_admin_adopts_the_password_init_set_and_closes_enrollment() {
        let dir = crate::private::test_dir("auth");
        let mut auth = Auth::load(&dir, true).unwrap();
        let file = auth.pending().expect("enrollment open");
        auth.adopt(Auth::read_hash(&file));
        assert!(auth.pending().is_some() && !auth.enrolled(), "nothing set yet");
        Auth::initialize(&dir, "correct horse battery").unwrap();
        auth.adopt(Auth::read_hash(&file));
        assert!(auth.enrolled() && auth.enrollment.is_none() && auth.pending().is_none());
        assert!(auth.login("correct horse battery").is_ok());
        let mut other = Auth::load(&crate::private::test_dir("auth"), true).unwrap();
        other.adopt(Err("holds no password hash".into()));
        assert!(other.enrollment.is_none() && !other.enrolled(), "a file it cannot use closes enrollment too");
        assert!(other.pending().is_none(), "and is not read again");
        // Enrollment gone (expired, or lost to `admin init`), the file is still read again.
        let dir = crate::private::test_dir("auth");
        let mut late = Auth::load(&dir, true).unwrap();
        late.enrollment = None;
        Auth::initialize(&dir, "correct horse battery").unwrap();
        let file = late.pending().expect("no password yet");
        late.adopt(Auth::read_hash(&file));
        assert!(late.enrolled() && late.login("correct horse battery").is_ok());
        assert!(Auth::load(&crate::private::test_dir("auth"), false).unwrap().pending().is_none(), "never in the demo");
    }

    /// The token and expiry the setup file holds.
    fn setup_of(dir: &Path) -> Setup {
        serde_json::from_slice(&std::fs::read(dir.join(SETUP)).unwrap()).unwrap()
    }

    #[test]
    fn the_setup_file_holds_a_valid_token_until_a_password_is_set() {
        let dir = crate::private::test_dir("auth");
        let mut auth = Auth::load(&dir, true).unwrap();
        let token = auth.enrollment.as_ref().unwrap().0.clone();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.join(SETUP)).unwrap().permissions().mode() & 0o777, 0o600);
        let setup = setup_of(&dir);
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(setup.token, token);
        assert!((now + 15 * 60 - 1..=now + 15 * 60).contains(&setup.expires), "{} for {now}", setup.expires);
        assert_eq!(Auth::setup_token(&dir, Duration::ZERO), Ok(SetupLink::Token(token.clone())));
        auth.renew();
        assert_eq!(setup_of(&dir).token, token, "a valid token stays");
        // Expired, and still no password: a new one replaces it, the old one sets nothing.
        auth.enrollment.as_mut().unwrap().1 -= ENROLLMENT_LIFETIME + Duration::from_secs(1);
        auth.renew();
        let renewed = auth.enrollment.as_ref().unwrap().0.clone();
        assert_ne!(renewed, token);
        assert_eq!(setup_of(&dir).token, renewed);
        assert_eq!(auth.enroll(&token, "correct horse battery"), Err(Refusal::Forbidden));
        // Setting the password logs in, as a login does, and removes the file.
        let (cookie, csrf) = auth.enroll(&renewed, "correct horse battery").unwrap();
        assert!(auth.is_live(&key_of(&cookie)) && !csrf.is_empty());
        assert!(!dir.join(SETUP).exists());
        assert_eq!(Auth::setup_token(&dir, Duration::ZERO), Ok(SetupLink::Set));
        auth.renew();
        assert!(!dir.join(SETUP).exists() && auth.enrollment.is_none(), "nothing is issued once one is set");
    }

    #[test]
    fn a_setup_file_is_removed_once_admin_init_set_the_password() {
        // Set while the admin runs (or while it is stopped): init removes the file, and the admin, taking the
        // password, issues no other.
        let dir = crate::private::test_dir("auth");
        let mut running = Auth::load(&dir, true).unwrap();
        assert!(dir.join(SETUP).exists());
        Auth::initialize(&dir, "correct horse battery").unwrap();
        assert!(!dir.join(SETUP).exists());
        running.adopt(Auth::read_hash(&dir.join("password")));
        running.renew();
        assert!(running.enrolled() && !dir.join(SETUP).exists());
        // One the admin wrote again before it read the password (a renewal just then) goes as it takes the password.
        let dir = crate::private::test_dir("auth");
        let mut running = Auth::load(&dir, true).unwrap();
        Auth::initialize(&dir, "correct horse battery").unwrap();
        std::fs::write(dir.join(SETUP), "{}").unwrap();
        running.adopt(Auth::read_hash(&dir.join("password")));
        assert!(!dir.join(SETUP).exists());
    }

    #[test]
    fn setup_link_waits_for_an_expired_token_to_be_replaced() {
        let dir = crate::private::test_dir("auth");
        let expired = Setup { token: "old".into(), expires: 1 };
        std::fs::write(dir.join(SETUP), serde_json::to_vec(&expired).unwrap()).unwrap();
        assert!(Auth::setup_token(&dir, Duration::from_millis(300)).unwrap_err().contains("no setup link is ready"));
        // The running admin's tick replaces it meanwhile.
        let writer = {
            let file = dir.join(SETUP);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                let fresh = Setup { token: "new".into(), expires: u64::MAX };
                crate::private::replace(&file, &serde_json::to_vec(&fresh).unwrap()).unwrap();
            })
        };
        assert_eq!(Auth::setup_token(&dir, Duration::from_secs(10)), Ok(SetupLink::Token("new".into())));
        writer.join().unwrap();
        std::fs::write(dir.join(SETUP), "not a setup").unwrap();
        assert!(Auth::setup_token(&dir, Duration::from_secs(10)).unwrap_err().contains("holds no setup link"));
        assert!(Auth::setup_token(&crate::private::test_dir("auth"), Duration::ZERO).is_err(), "none at all");
    }

    #[test]
    fn the_demo_never_enrolls_and_opens_logins_without_a_password() {
        let dir = crate::private::test_dir("auth");
        let mut auth = Auth::load(&dir, false).unwrap();
        assert!(auth.enrollment.is_none() && !auth.enrolled());
        auth.renew();
        assert!(!dir.join(SETUP).exists(), "and writes no setup file");
        assert_eq!(auth.check_enrollment("anything", "correct horse battery"), Err(Refusal::Forbidden));
        assert_eq!(auth.login(""), Err(Refusal::Forbidden), "no password login either");
        let (cookie, csrf) = auth.open_login();
        assert!(auth.is_live(&key_of(&cookie)) && !csrf.is_empty());
    }
}
