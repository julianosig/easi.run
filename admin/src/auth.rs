//! The one person who administers this installation.
//!
//! While no password is set (outside the demo), the admin's first screen sets it, from the exact configured origin
//! like every other change; `admin init` sets it instead, from its stdin, before or while the admin runs. Neither
//! replaces a password that is set.
//! Passwords are hashed with Argon2id. A login (setting the password opens one too) creates a session: a `__Host-`
//! cookie that is `Secure`, `HttpOnly`, and `SameSite=Strict`, with an idle and an absolute expiry, and a CSRF token
//! every state-changing request must echo. Every state-changing request and every WebSocket upgrade must also come
//! from the exact configured origin. With the second step on (see `second_step`), a right password opens no login
//! but a pending step, which a code then answers; wrong passwords and wrong codes count toward the same backoff, and
//! only an opened login resets it, so knowing the password never buys more guesses at a code. Turning the second
//! step on, off, or replacing what it holds ends every other login.
//!
//! Outside the demo, every password checked right also derives the key that unlocks the saved profiles (see `keys`),
//! held here, in memory only, from the login it opens (setting the password opens one too) until the admin stops: a
//! pending second step holds it until its code opens the login. A passkey derives none, so after a restart a passkey
//! sign-in leaves the profiles locked until the password is entered once (see [`Auth::unlock`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::http::{HeaderMap, Method, header};
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::keys::{self, Kek};
use crate::private::if_there;
use crate::second_step::{Answered, Methods, SecondStep};

pub const COOKIE: &str = "__Host-display";
pub const CSRF_HEADER: &str = "x-csrf-token";
/// The salt the profiles' KEK is derived with, in the state directory beside the password hash (see `keys`).
const KEY_SALT: &str = "key-salt";
pub const IDLE_EXPIRY: Duration = Duration::from_secs(30 * 60);
pub const ABSOLUTE_EXPIRY: Duration = Duration::from_secs(12 * 60 * 60);
const MIN_PASSWORD: usize = 12;
pub const MAX_PASSWORD: usize = 1024;
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
    hash: Option<String>,
    /// Whether the password file is read again while no password is set (`admin init` may set one): outside the demo,
    /// until it holds a password or one it cannot use.
    rereads: bool,
    sessions: HashMap<LoginKey, Login>,
    failures: u32,
    locked_until: Option<Instant>,
    /// Password checks running now: each costs an Argon2id hash on a blocking thread.
    verifying: u32,
    /// The second step, outside the demo.
    second: Option<SecondStep>,
    /// The salt of the profiles' KEK, outside the demo.
    salt: Option<[u8; keys::SALT]>,
    /// The profiles' KEK, once a password checked right since the start derived it.
    kek: Option<Kek>,
}

/// What a right password opens (see [`Auth::finish_login`]).
#[derive(Debug, PartialEq, Eq)]
pub enum Opened {
    /// A login: its cookie value and CSRF token.
    Login(String, String),
    /// The second step is on: a pending step's token, and what may answer it.
    SecondStep(String, Methods),
}

/// What `admin init` found (see [`Auth::initialize`]).
#[derive(Debug, PartialEq, Eq)]
pub enum Initialized {
    /// The password is the one given: set now, or set to it already.
    Set,
    /// Another password was set already: it is left as it was.
    Other,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Unauthenticated,
    Forbidden,
    /// The request came from another origin than the admin's own, which it names: the page is open at another address
    /// (`https://127.0.0.1:9443` for `https://localhost:9443`, another tunnel port, the server's IP).
    WrongOrigin(String),
    BadRequest(String),
    TooManyAttempts,
    /// The pending step after the password is gone (used, expired, or dropped after too many codes): the password
    /// is asked again.
    Again,
    /// A change behind the login was asked with a password that is not the current one.
    WrongPassword,
    /// The server could not do what was asked (a write that failed): nothing was changed.
    Failed(&'static str),
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// The SHA-256 of `text`.
pub fn sha256(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// `bytes` in lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The 32 bytes 64 hex digits spell, and nothing else.
pub fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// The time now, in whole seconds since the Unix epoch.
pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The login a cookie value names.
pub fn key_of(cookie_value: &str) -> LoginKey {
    LoginKey(sha256(cookie_value))
}

fn equal(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

/// The rule for a new password, which enrollment and `admin init` both apply: [`MIN_PASSWORD`] to [`MAX_PASSWORD`]
/// characters (code points), or what it needs.
fn password_length(password: &str) -> Result<(), String> {
    if (MIN_PASSWORD..=MAX_PASSWORD).contains(&password.chars().count()) {
        Ok(())
    } else {
        Err(format!("the password needs {MIN_PASSWORD} to {MAX_PASSWORD} characters"))
    }
}

impl Auth {
    /// Loads the password hash, or opens enrollment. The public demo has no password and never enrolls (`enrollable`
    /// false): its visitors get in without one. Only a password file that is not there opens enrollment: one that
    /// cannot be read, or holds no hash Argon2 can verify, stops the start, since enrolling would replace the
    /// administrator's password, and a hash it cannot verify would fail every login. Outside the demo, a second step file that cannot be read whole stops it too.
    pub fn load(state_dir: &Path, enrollable: bool) -> Result<Auth, String> {
        let password_file = state_dir.join("password");
        let hash = Auth::read_hash(&password_file)?;
        let second = if enrollable { Some(SecondStep::load(state_dir)?) } else { None };
        let salt = if enrollable { Some(load_salt(&state_dir.join(KEY_SALT))?) } else { None };
        let auth = Auth {
            password_file,
            hash,
            rereads: enrollable,
            sessions: HashMap::new(),
            failures: 0,
            locked_until: None,
            verifying: 0,
            second,
            salt,
            kek: None,
        };
        if auth.pending().is_some() {
            tracing::warn!("no password is set; the admin's first screen sets it");
        }
        Ok(auth)
    }

    /// The hash in a password file: none when the file is not there, an error when it cannot be read or holds no hash
    /// Argon2 can verify.
    pub fn read_hash(password_file: &Path) -> Result<Option<String>, String> {
        let read = if_there(std::fs::read_to_string(password_file));
        let Some(text) = read.map_err(|e| format!("{} cannot be read: {e}", password_file.display()))? else {
            return Ok(None);
        };
        let hash = text.trim().to_owned();
        // Checked by verifying a password it cannot hold (the shortest allowed is 12 characters): only a whole hash
        // Argon2 can verify answers that it does not match, rather than failing to read it.
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

    pub fn enrolled(&self) -> bool {
        self.hash.is_some()
    }

    /// The password file to read again (with [`Auth::read_hash`], off the lock) while no password is set, which is
    /// while enrollment is open: `admin init` may have set one since the start.
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
                self.hash = Some(hash);
                tracing::info!("a password was set (admin init); enrollment closed");
            }
            Err(why) => {
                self.rereads = false;
                tracing::warn!(%why, "enrollment closed");
            }
        }
    }

    /// Enrollment, step 1: no password is set (outside the demo), and the password's length is allowed.
    pub fn check_enrollment(&self, password: &str) -> Result<(), Refusal> {
        if self.pending().is_none() {
            return Err(Refusal::Forbidden);
        }
        password_length(password).map_err(Refusal::BadRequest)
    }

    /// Enrollment, step 2 (slow, no lock): the password's Argon2id hash.
    pub fn hash_password(password: &str) -> Result<String, Refusal> {
        let salt = SaltString::encode_b64(&rand::random::<[u8; 16]>()).expect("a 16-byte salt encodes");
        argon2::Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|_| Refusal::BadRequest("the password could not be hashed".into()))
    }

    /// Enrollment, step 3: stores the hash if enrollment is still open (another request may have set a password
    /// meanwhile) and no password file is there (`admin init` may have written one; it is never replaced), then opens
    /// a login, as a login does, unlocking the profiles with the KEK the password derived, and returns its cookie value
    /// and CSRF token. A password file that could not be written (a full disk, a failing one) sets nothing, so
    /// enrollment stays open for another try, and the failure is logged and answered as one, never as a refusal. One
    /// written whose directory could not be synced is set all the same, and logged.
    pub fn complete_enrollment(&mut self, hash: String, unlock: Option<Kek>) -> Result<(String, String), Refusal> {
        if self.pending().is_none() {
            return Err(Refusal::Forbidden);
        }
        let created = match crate::private::create(&self.password_file, hash.as_bytes()) {
            Ok(created) => created,
            // A failure after the file was linked in place (its directory's sync) left a password set: this one, or
            // another `admin init` set first.
            Err(why) => match Auth::read_hash(&self.password_file) {
                Ok(Some(found)) => {
                    tracing::warn!(%why, "a password is set, but its directory could not be synced");
                    found == hash
                }
                _ => {
                    tracing::error!(%why, "the password could not be written; enrollment stays open");
                    return Err(Refusal::Failed("the password could not be saved on the server; its log says why"));
                }
            },
        };
        if !created {
            // `admin init` set one meanwhile: taken now rather than at the next reread, so the page, asking at once,
            // hears that a password is set and offers the login.
            self.adopt(Auth::read_hash(&self.password_file));
            return Err(Refusal::Forbidden);
        }
        self.hash = Some(hash);
        tracing::info!("password set; enrollment closed");
        self.unlock_with(unlock);
        Ok(self.open_login())
    }

    /// `admin init`: sets the password from `password` unless one is set already, exactly as enrollment stores it, in
    /// the password file of `state_dir`, which it never replaces; a password set already is checked against
    /// `password`. A password file that cannot be read or holds no hash is an error, as at the start.
    pub fn initialize(state_dir: &Path, password: &str) -> Result<Initialized, String> {
        password_length(password)?;
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
            Ok(Initialized::Set)
        } else {
            // Another one was set meanwhile.
            set(Auth::read_hash(&password_file)?)
        }
    }

    /// Whether the backoff holds every attempt off now.
    fn locked(&self) -> bool {
        self.locked_until.is_some_and(|t| Instant::now() < t)
    }

    /// Counts a wrong password or code toward the backoff.
    fn failed(&mut self) {
        self.failures += 1;
        if self.failures >= FREE_FAILURES {
            let wait = Duration::from_secs(2u64.pow((self.failures + 1 - FREE_FAILURES).min(8)));
            self.locked_until = Some(Instant::now() + wait);
        }
    }

    /// Opens a login once every step is passed, which alone resets the backoff.
    fn signed_in(&mut self) -> (String, String) {
        self.failures = 0;
        self.locked_until = None;
        self.open_login()
    }

    /// Login, step 1: not locked out and not too many checks running; returns the stored hash to verify against.
    /// A password asked for a change behind the login takes the same three steps, and the same backoff.
    pub fn begin_login(&mut self) -> Result<String, Refusal> {
        if self.locked() || self.verifying >= MAX_VERIFYING {
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

    /// Login, step 3: counts a failure toward the backoff; or, the password right, opens a login unlocking the
    /// profiles with `unlock`, the KEK it derived, or with the second step on a pending step that holds it, which leaves
    /// the backoff as it is.
    pub fn finish_login(&mut self, matched: bool, unlock: Option<Kek>) -> Result<Opened, Refusal> {
        self.verifying = self.verifying.saturating_sub(1);
        if !matched {
            self.failed();
            return Err(Refusal::Unauthenticated);
        }
        if let Some(second) = self.second.as_mut()
            && let Some((step, methods)) = second.after_password(unlock.clone())?
        {
            return Ok(Opened::SecondStep(step, methods));
        }
        self.unlock_with(unlock);
        let (cookie, csrf) = self.signed_in();
        Ok(Opened::Login(cookie, csrf))
    }

    /// The salt the profiles' KEK is derived with, outside the demo.
    pub fn salt(&self) -> Option<[u8; keys::SALT]> {
        self.salt
    }

    /// The profiles' KEK, unless no password was checked right since the start.
    pub fn kek(&self) -> Option<Kek> {
        self.kek.clone()
    }

    /// Unlocks the profiles with the KEK a password checked behind a login derived (see `api`): after a passkey
    /// sign-in, which derives none.
    pub fn unlock(&mut self, kek: Kek) {
        self.kek = Some(kek);
    }

    /// Unlocks the profiles with `unlock`, when a password derived one.
    fn unlock_with(&mut self, unlock: Option<Kek>) {
        if let Some(kek) = unlock {
            self.unlock(kek);
        }
    }

    /// The pending step `step` answered with `code` (from the authenticator app, or a recovery code): a login, or a
    /// wrong code counted toward the backoff.
    pub fn sign_in_with_code(&mut self, step: &str, code: &str) -> Result<(String, String), Refusal> {
        if self.locked() {
            return Err(Refusal::TooManyAttempts);
        }
        match self.second_mut()?.answer(step, code)? {
            Answered::Right(unlock) => {
                self.unlock_with(unlock);
                Ok(self.signed_in())
            }
            Answered::Wrong => {
                self.failed();
                Err(Refusal::Unauthenticated)
            }
            // The step is gone: the person is told to enter the password again rather than another code.
            Answered::WrongLast => {
                self.failed();
                Err(Refusal::Again)
            }
        }
    }

    /// Offers passkeys at `origin` (`ADMIN_ORIGIN`), outside the demo (see `second_step`).
    pub fn offer_passkeys(&mut self, origin: &str) {
        if let Some(second) = self.second.as_mut() {
            second.offer_passkeys(origin);
        }
    }

    /// Whether the login screen offers a passkey.
    pub fn passkey_sign_in(&self) -> bool {
        self.second.as_ref().is_some_and(SecondStep::passkey_sign_in)
    }

    /// Begins a passkey sign-in, unless the backoff holds attempts off: its token and the browser's options.
    pub fn begin_passkey_sign_in(&mut self) -> Result<(String, serde_json::Value), Refusal> {
        if self.locked() {
            return Err(Refusal::TooManyAttempts);
        }
        self.second_mut()?.begin_passkey_sign_in()
    }

    /// Finishes a passkey sign-in: a login, in place of the password and a code, or a refusal counted toward the
    /// backoff.
    pub fn finish_passkey_sign_in(
        &mut self,
        ceremony: &str,
        credential: &webauthn_rs::prelude::PublicKeyCredential,
    ) -> Result<(String, String), Refusal> {
        if self.locked() {
            return Err(Refusal::TooManyAttempts);
        }
        if self.second_mut()?.finish_passkey_sign_in(ceremony, credential)? {
            Ok(self.signed_in())
        } else {
            self.failed();
            Err(Refusal::Unauthenticated)
        }
    }

    /// A password asked for a change behind the login, step 3 (after [`Auth::begin_login`] and
    /// [`Auth::verify_password`]): a wrong one counts toward the backoff, as at a login.
    pub fn finish_password_check(&mut self, matched: bool) -> Result<(), Refusal> {
        self.verifying = self.verifying.saturating_sub(1);
        if matched {
            Ok(())
        } else {
            self.failed();
            Err(Refusal::WrongPassword)
        }
    }

    /// The second step, outside the demo.
    pub fn second_mut(&mut self) -> Result<&mut SecondStep, Refusal> {
        self.second.as_mut().ok_or(Refusal::Forbidden)
    }

    /// Ends every login but `keep`, and every pending step, after the second step changed; returns the logins ended,
    /// so their streams can be closed.
    pub fn end_other_logins(&mut self, keep: LoginKey) -> Vec<LoginKey> {
        let ended = self.sessions.extract_if(|key, _| *key != keep).map(|(key, _)| key).collect();
        if let Some(second) = self.second.as_mut() {
            second.forget_pending();
        }
        ended
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
    pub fn enroll(&mut self, password: &str) -> Result<(String, String), Refusal> {
        self.check_enrollment(password)?;
        let hash = Auth::hash_password(password)?;
        let unlock = self.salt.map(|salt| keys::derive(password, &salt));
        self.complete_enrollment(hash, unlock)
    }

    /// All three login steps at once, for tests: a login, with the second step off.
    #[cfg(test)]
    pub fn login(&mut self, password: &str) -> Result<(String, String), Refusal> {
        match self.sign_in(password)? {
            Opened::Login(cookie, csrf) => Ok((cookie, csrf)),
            Opened::SecondStep(..) => panic!("the second step is on"),
        }
    }

    /// All three login steps at once, for tests.
    #[cfg(test)]
    pub fn sign_in(&mut self, password: &str) -> Result<Opened, Refusal> {
        let hash = self.begin_login()?;
        let matched = Auth::verify_password(&hash, password);
        let unlock = matched.then_some(self.salt).flatten().map(|salt| keys::derive(password, &salt));
        self.finish_login(matched, unlock)
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
                return Err(Refusal::WrongOrigin(origin.to_owned()));
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
            return Err(Refusal::WrongOrigin(origin.to_owned()));
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
        self.sessions.extract_if(|_, l| !live(l, now)).map(|(key, _)| key).collect()
    }
}

/// The KEK's salt in `file`, drawn and written there first when it is not there. A file that cannot be read, or holds
/// no salt, stops the start: a new salt would derive another KEK, which opens none of the profiles.
fn load_salt(file: &Path) -> Result<[u8; keys::SALT], String> {
    let read = |file: &Path| match if_there(std::fs::read(file)) {
        Ok(Some(bytes)) => bytes
            .try_into()
            .map(Some)
            .map_err(|_| format!("{} holds no salt; restore it, or remove it with the saved profiles", file.display())),
        Ok(None) => Ok(None),
        Err(e) => Err(format!("{} cannot be read: {e}", file.display())),
    };
    if let Some(salt) = read(file)? {
        return Ok(salt);
    }
    // Created only where none is, so a salt is never replaced; one written meanwhile is read instead.
    crate::private::create(file, &keys::new_salt())?;
    read(file)?.ok_or_else(|| format!("{} was not written", file.display()))
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

    /// The admin of `dir` once its password is set to "correct horse battery".
    fn enrolled(dir: &Path) -> Auth {
        let mut auth = Auth::load(dir, true).unwrap();
        auth.enroll("correct horse battery").unwrap();
        auth
    }

    #[test]
    fn parallel_password_checks_are_bounded() {
        let dir = crate::private::test_dir("auth");
        let mut auth = enrolled(&dir);
        let hash = auth.begin_login().unwrap();
        auth.begin_login().unwrap();
        assert_eq!(auth.begin_login(), Err(Refusal::TooManyAttempts), "a third check at once is refused");
        assert!(auth.finish_login(Auth::verify_password(&hash, "correct horse battery"), None).is_ok());
        assert!(auth.begin_login().is_ok(), "a finished check frees its place");
    }

    #[test]
    fn enrollment_sets_the_first_password_only() {
        let dir = crate::private::test_dir("auth");
        let mut auth = Auth::load(&dir, true).unwrap();
        assert!(matches!(auth.enroll("short"), Err(Refusal::BadRequest(_))));
        let checked = auth.check_enrollment("a password checked first");
        auth.enroll("correct horse battery").unwrap();
        assert_eq!(auth.enroll("another long password"), Err(Refusal::Forbidden));
        // One checked before the first was set (its hash made meanwhile) sets nothing either.
        assert!(checked.is_ok());
        let hash = Auth::hash_password("a password checked first").unwrap();
        assert_eq!(auth.complete_enrollment(hash, None), Err(Refusal::Forbidden));
        assert!(auth.login("correct horse battery").is_ok());
        // The hash persists and enrollment stays closed after a restart.
        let again = Auth::load(&dir, true).unwrap();
        assert!(again.enrolled() && again.pending().is_none());
    }

    #[test]
    fn a_password_that_cannot_be_written_is_a_failure_that_keeps_enrollment_open() {
        let dir = crate::private::test_dir("auth");
        let mut auth = Auth::load(&dir, true).unwrap();
        // A directory that is not there stands for a disk that fails the write.
        let file = std::mem::replace(&mut auth.password_file, dir.join("missing/password"));
        assert!(matches!(auth.enroll("correct horse battery"), Err(Refusal::Failed(_))));
        assert!(!auth.enrolled());
        auth.password_file = file;
        assert!(auth.enroll("correct horse battery").is_ok(), "it works once the write can");
        assert!(auth.enrolled() && auth.login("correct horse battery").is_ok());
    }

    #[test]
    fn requests_need_cookie_origin_and_csrf() {
        let dir = crate::private::test_dir("auth");
        let mut auth = enrolled(&dir);
        assert_eq!(auth.login("nope nope nope"), Err(Refusal::Unauthenticated));
        let (cookie, csrf) = auth.login("correct horse battery").unwrap();
        assert!(auth.check(&Method::GET, &headers(Some(&cookie), None, None), ORIGIN).is_ok());
        assert_eq!(auth.check(&Method::GET, &headers(None, None, None), ORIGIN).unwrap_err(), Refusal::Unauthenticated);
        let mut post = |h: HeaderMap| auth.check(&Method::POST, &h, ORIGIN).map(|_| ());
        let elsewhere = Err(Refusal::WrongOrigin(ORIGIN.into()));
        assert_eq!(post(headers(Some(&cookie), Some(ORIGIN), None)), Err(Refusal::Forbidden));
        assert_eq!(post(headers(Some(&cookie), Some("https://evil.example"), Some(&csrf))), elsewhere);
        assert_eq!(post(headers(Some(&cookie), None, Some(&csrf))), elsewhere);
        assert!(post(headers(Some(&cookie), Some(ORIGIN), Some(&csrf))).is_ok());
        assert_eq!(
            auth.check_upgrade(&headers(Some(&cookie), Some("https://evil.example"), None), ORIGIN),
            Err(Refusal::WrongOrigin(ORIGIN.into()))
        );
        let key = auth.check_upgrade(&headers(Some(&cookie), Some(ORIGIN), None), ORIGIN).unwrap();
        auth.logout(key);
        assert_eq!(
            auth.check(&Method::GET, &headers(Some(&cookie), None, None), ORIGIN).unwrap_err(),
            Refusal::Unauthenticated
        );
    }

    #[test]
    fn idle_and_absolute_expiry_end_sessions() {
        let dir = crate::private::test_dir("auth");
        let mut auth = enrolled(&dir);
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
        let mut auth = enrolled(&dir);
        for _ in 0..FREE_FAILURES {
            assert_eq!(auth.login("wrong password!!"), Err(Refusal::Unauthenticated));
        }
        assert_eq!(auth.login("correct horse battery"), Err(Refusal::TooManyAttempts));
    }

    /// A login with the authenticator app on, and its recovery codes.
    fn with_app(dir: &Path) -> (Auth, Vec<String>) {
        let mut auth = Auth::load(dir, true).unwrap();
        let (cookie, _) = auth.enroll("correct horse battery").unwrap();
        let second = auth.second_mut().unwrap();
        second.begin_app(key_of(&cookie));
        let code = second.test_code(0);
        let codes = second.confirm_app(key_of(&cookie), &code, false).unwrap().unwrap();
        (auth, codes)
    }

    fn pending(auth: &mut Auth) -> String {
        match auth.sign_in("correct horse battery") {
            Ok(Opened::SecondStep(step, methods)) => {
                assert_eq!(methods, Methods { app: true, passkey: false });
                step
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn with_the_second_step_on_the_password_alone_opens_no_login() {
        let dir = crate::private::test_dir("auth");
        let (mut auth, codes) = with_app(&dir);
        let step = pending(&mut auth);
        assert_eq!(auth.sessions.len(), 1, "only the enrollment's login");
        let code = auth.second_mut().unwrap().test_code(1);
        let (cookie, _) = auth.sign_in_with_code(&step, &code).unwrap();
        assert!(auth.is_live(&key_of(&cookie)));
        assert_eq!(auth.sign_in_with_code(&step, &codes[0]), Err(Refusal::Again), "the step is used");
        let step = pending(&mut auth);
        assert!(auth.sign_in_with_code(&step, &codes[0]).is_ok(), "a recovery code instead");
    }

    #[test]
    fn wrong_codes_count_toward_the_backoff_and_a_right_password_never_resets_it() {
        let dir = crate::private::test_dir("auth");
        let (mut auth, _) = with_app(&dir);
        let right = auth.second_mut().unwrap().test_code(1);
        let wrong = if right == "123456" { "654321" } else { "123456" };
        for _ in 0..FREE_FAILURES {
            let step = pending(&mut auth);
            assert_eq!(auth.sign_in_with_code(&step, wrong), Err(Refusal::Unauthenticated));
        }
        assert_eq!(auth.sign_in("correct horse battery"), Err(Refusal::TooManyAttempts), "the password waits too");
        let step = {
            auth.locked_until = None;
            pending(&mut auth)
        };
        assert_eq!(auth.failures, FREE_FAILURES, "a right password leaves the count as it was");
        auth.locked_until = Some(Instant::now() + Duration::from_secs(60));
        assert_eq!(auth.sign_in_with_code(&step, &right), Err(Refusal::TooManyAttempts), "and so does the code");
        auth.locked_until = None;
        assert!(auth.sign_in_with_code(&step, &right).is_ok());
        assert_eq!((auth.failures, auth.locked_until), (0, None), "only an opened login resets it");
    }

    #[test]
    fn a_wrong_password_for_a_change_counts_toward_the_backoff() {
        let dir = crate::private::test_dir("auth");
        let (mut auth, _) = with_app(&dir);
        for _ in 0..FREE_FAILURES {
            auth.begin_login().unwrap();
            assert_eq!(auth.finish_password_check(false), Err(Refusal::WrongPassword));
        }
        assert_eq!(auth.begin_login(), Err(Refusal::TooManyAttempts));
    }

    #[test]
    fn a_change_of_the_second_step_ends_every_other_login_and_pending_step() {
        let dir = crate::private::test_dir("auth");
        let (mut auth, codes) = with_app(&dir);
        let mine = key_of(&auth.open_login().0);
        let other = key_of(&auth.open_login().0);
        let step = pending(&mut auth);
        let ended = auth.end_other_logins(mine);
        assert!(ended.contains(&other) && !ended.contains(&mine));
        assert!(auth.is_live(&mine) && !auth.is_live(&other));
        assert_eq!(auth.sign_in_with_code(&step, &codes[0]), Err(Refusal::Again));
        assert_eq!(
            Auth::load(&crate::private::test_dir("auth"), false).unwrap().second_mut().err(),
            Some(Refusal::Forbidden),
            "none in the demo"
        );
    }

    #[test]
    fn a_right_password_unlocks_the_profiles_until_the_admin_restarts() {
        let dir = crate::private::test_dir("auth");
        let auth = enrolled(&dir);
        let salt = auth.salt().expect("a salt outside the demo");
        let kek = keys::derive("correct horse battery", &salt);
        assert_eq!(auth.kek(), Some(kek.clone()), "setting the password unlocks them");
        let mut restarted = Auth::load(&dir, true).unwrap();
        assert_eq!(restarted.salt(), Some(salt), "the salt is kept beside the password hash");
        assert_eq!(restarted.kek(), None, "locked after a restart");
        assert_eq!(restarted.login("wrong password!!"), Err(Refusal::Unauthenticated));
        assert_eq!(restarted.kek(), None, "a wrong password unlocks nothing");
        let (cookie, _) = restarted.login("correct horse battery").unwrap();
        assert_eq!(restarted.kek(), Some(kek.clone()), "the next login unlocks them");
        restarted.logout(key_of(&cookie));
        assert_eq!(restarted.kek(), Some(kek), "and they stay unlocked while the admin runs");
        std::fs::write(dir.join(KEY_SALT), b"short").unwrap();
        assert!(Auth::load(&dir, true).err().unwrap().contains("salt"), "a salt it cannot use stops the start");
        let demo = Auth::load(&crate::private::test_dir("auth"), false).unwrap();
        assert!(demo.salt().is_none() && demo.kek().is_none(), "the demo has no profiles");
    }

    #[test]
    fn with_the_second_step_on_the_code_unlocks_the_profiles_with_the_password_s_key() {
        let dir = crate::private::test_dir("auth");
        let (auth, _) = with_app(&dir);
        drop(auth);
        let mut auth = Auth::load(&dir, true).unwrap();
        let step = pending(&mut auth);
        assert_eq!(auth.kek(), None, "a right password alone opens no login and unlocks nothing");
        let code = auth.second_mut().unwrap().test_code(1);
        auth.sign_in_with_code(&step, &code).unwrap();
        let kek = keys::derive("correct horse battery", &auth.salt().unwrap());
        assert_eq!(auth.kek(), Some(kek.clone()), "the code opens the login and unlocks them");
        let mut passkey_only = Auth::load(&dir, true).unwrap();
        passkey_only.signed_in();
        assert_eq!(passkey_only.kek(), None, "a sign-in without the password leaves them locked");
        passkey_only.unlock(kek.clone());
        assert_eq!(passkey_only.kek(), Some(kek), "until the password unlocks them");
    }

    #[test]
    fn a_login_dies_at_its_deadline() {
        let dir = crate::private::test_dir("auth");
        let mut auth = enrolled(&dir);
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
        enrolled(&dir);
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
        assert!(Auth::initialize(&dir, "short").is_err(), "12 characters at least");
        assert_eq!(Auth::initialize(&dir, "correct horse battery"), Ok(Initialized::Set));
        assert_eq!(Auth::initialize(&dir, "correct horse battery"), Ok(Initialized::Set), "the same one again");
        assert_eq!(Auth::initialize(&dir, "another long password"), Ok(Initialized::Other));
        let set = std::fs::read_to_string(dir.join("password")).unwrap();
        // An admin that started before it still has enrollment open: completing it leaves the password alone, and
        // takes the one set at once.
        assert_eq!(running.enroll("an enrolled password"), Err(Refusal::Forbidden));
        assert_eq!(std::fs::read_to_string(dir.join("password")).unwrap(), set);
        assert!(running.enrolled() && running.login("correct horse battery").is_ok());
        let mut auth = Auth::load(&dir, true).unwrap();
        assert!(auth.pending().is_none());
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
        assert!(auth.enrolled() && auth.pending().is_none());
        assert!(auth.login("correct horse battery").is_ok());
        let mut other = Auth::load(&crate::private::test_dir("auth"), true).unwrap();
        other.adopt(Err("holds no password hash".into()));
        assert!(other.pending().is_none() && !other.enrolled(), "a file it cannot use closes enrollment too");
        assert_eq!(other.check_enrollment("correct horse battery"), Err(Refusal::Forbidden));
        assert!(Auth::load(&crate::private::test_dir("auth"), false).unwrap().pending().is_none(), "never in the demo");
    }

    #[test]
    fn the_demo_never_enrolls_and_opens_logins_without_a_password() {
        let dir = crate::private::test_dir("auth");
        let mut auth = Auth::load(&dir, false).unwrap();
        assert!(auth.pending().is_none() && !auth.enrolled());
        assert_eq!(auth.check_enrollment("correct horse battery"), Err(Refusal::Forbidden));
        assert_eq!(auth.login(""), Err(Refusal::Forbidden), "no password login either");
        let (cookie, csrf) = auth.open_login();
        assert!(auth.is_live(&key_of(&cookie)) && !csrf.is_empty());
    }
}
