//! The optional second step at sign-in, outside the public demo: once it is on, the password alone opens no login.
//! The second step is a code from an authenticator app (see `totp`), or one of the recovery codes issued when the
//! second step is turned on: ten, of 80 random bits each, each good for one sign-in, of which only the SHA-256 is
//! kept (a code that long is never guessed, so a slow hash would add nothing).
//!
//! Passkeys (WebAuthn, verified by the webauthn-rs crate) sign in on their own, in place of the password and a code:
//! a passkey is two factors already, the device and the PIN or biometric that unlocks it, which every ceremony
//! requires (user verification). Their relying party is the admin's own origin, `ADMIN_ORIGIN`: its host is the RP
//! ID, and only that exact origin (scheme, host and port) is accepted. An origin whose host is an IP address has no
//! RP ID, so it offers no passkeys. A ceremony's challenge is kept in memory only, and used once; adding a passkey
//! takes the current password, since a passkey alone opens a login.
//!
//! What is on is kept in `second-step.json` in the state directory (mode 0600, replaced atomically) and read again at
//! every use, so `admin reset-second-step`, which removes the file from another process, takes effect at once. A file
//! that cannot be read whole stops the start and refuses every sign-in until it is restored or removed: it never turns
//! the second step off. A change that cannot be written changes nothing, and a code whose use cannot be written opens
//! no login, so a code is never good twice. Every read that leads to a write, and the reset's removal, hold an
//! exclusive lock on `second-step.lock` beside it, so a reset never loses to a change the server was making.
//!
//! A sign-in whose password was right while the second step is on gets a pending step instead of a login: a random
//! token, kept only as its SHA-256, good for [`STEP_LIFETIME`] and one login, with at most [`STEP_ATTEMPTS`] codes
//! tried on it. Turning the authenticator app on takes two requests: the first draws a secret that does nothing yet,
//! the second turns it on once a code from the app shows it was added there.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential, Url, Uuid,
    Webauthn, WebauthnBuilder,
};

use crate::auth::{LoginKey, Refusal, sha256, unix_now};
use crate::keys::Kek;
use crate::private::if_there;
use crate::totp;

/// The file, in the state directory.
pub const FILE: &str = "second-step.json";
/// The lock every change of it holds (see [`lock`]).
const LOCK: &str = "second-step.lock";
/// How long the pending step after a right password lasts.
const STEP_LIFETIME: Duration = Duration::from_secs(2 * 60);
/// Codes tried on one pending step before it is dropped: the password is asked again.
const STEP_ATTEMPTS: u32 = 5;
/// Pending steps at once; a new one drops the oldest.
const MAX_STEPS: usize = 8;
/// How long a secret drawn for the authenticator app waits for the code that turns it on.
const SETUP_LIFETIME: Duration = Duration::from_secs(10 * 60);
const RECOVERY_CODES: usize = 10;
/// A recovery code's characters, 5 bits each: Crockford's base32, which leaves out the letters read as digits.
const RECOVERY_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
const RECOVERY_LENGTH: usize = 16;
/// How long a passkey ceremony may take: the default timeout webauthn-rs asks the browser for.
const CEREMONY_LIFETIME: Duration = Duration::from_secs(5 * 60);
/// Passkey sign-ins begun at once (anyone reaching the admin may begin one); a new one drops the oldest.
const MAX_CEREMONIES: usize = 8;
/// Passkeys at once.
const MAX_PASSKEYS: usize = 16;

/// What is on, as the file holds it.
#[derive(Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Stored {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app: Option<AppFactor>,
    /// The SHA-256 of each recovery code not used yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    recovery: Vec<[u8; 32]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    passkeys: Vec<StoredPasskey>,
    /// The id the passkeys know the administrator by (WebAuthn's user handle): random, drawn with the first one and
    /// kept, so a device that holds an earlier passkey of this admin replaces it rather than keeping two.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user: Option<Uuid>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPasskey {
    /// The name the API removes it by.
    id: String,
    /// When it was added, in Unix seconds.
    created: u64,
    /// Its public key, signature counter and flags, as webauthn-rs keeps them.
    passkey: Passkey,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppFactor {
    secret: [u8; totp::SECRET_BYTES],
    /// The step of the last code taken: it and every earlier one are refused from then on.
    last_step: u64,
}

impl Stored {
    fn on(&self) -> bool {
        self.app.is_some() || !self.passkeys.is_empty()
    }

    /// New recovery codes in place of any left, returned as the person is shown them.
    fn new_recovery_codes(&mut self) -> Vec<String> {
        let codes: Vec<String> = (0..RECOVERY_CODES).map(|_| recovery_code()).collect();
        self.recovery = codes.iter().map(|code| sha256(&normalized(code))).collect();
        codes
    }

    /// Takes `code` out if it is one of the recovery codes left.
    fn use_recovery_code(&mut self, code: &str) -> bool {
        let hash = sha256(&normalized(code));
        // Every stored hash is compared, in constant time, whichever matches.
        let found = self
            .recovery
            .iter()
            .fold(None, |found, stored| if bool::from(stored.ct_eq(&hash)) { Some(*stored) } else { found });
        self.recovery.retain(|stored| Some(*stored) != found);
        found.is_some()
    }
}

/// A recovery code as shown: four groups of four, `xxxx-xxxx-xxxx-xxxx`.
fn recovery_code() -> String {
    let mut bytes = [0u8; RECOVERY_LENGTH];
    rand::rng().fill_bytes(&mut bytes);
    let chars: Vec<char> = bytes.iter().map(|b| RECOVERY_ALPHABET[usize::from(b & 31)] as char).collect();
    chars.chunks(4).map(|group| group.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

/// A recovery code as typed, reduced to the characters that count: lowercase, without spaces or dashes, and with the
/// letters people type for the digits they look like read as those digits.
fn normalized(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| match c.to_ascii_lowercase() {
            'o' => '0',
            'i' | 'l' => '1',
            c => c,
        })
        .collect()
}

/// The signature counter stored for `passkey`, which webauthn-rs keeps but does not hand out.
fn counter(passkey: &Passkey) -> u32 {
    let stored = serde_json::to_value(passkey).expect("a passkey serializes");
    stored["cred"]["counter"].as_u64().and_then(|c| u32::try_from(c).ok()).expect("a passkey has its counter")
}

/// An exclusive lock on the second step of the state directory `file` is in, held until the file returned is dropped:
/// the server's changes and `admin reset-second-step` each read and write under it.
fn lock(file: &Path) -> Result<std::fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = file.with_file_name(LOCK);
    let locked =
        std::fs::OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).open(&path).and_then(|lock| {
            rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive)?;
            Ok(lock)
        });
    locked.map_err(|e| format!("{}: {e}", path.display()))
}

/// What `file` holds: nothing on when it is not there.
fn read(file: &Path) -> Result<Stored, String> {
    match crate::private::read(file)? {
        Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| {
            format!("{} holds no second step; restore it, or remove it with admin reset-second-step", file.display())
        }),
        None => Ok(Stored::default()),
    }
}

/// `admin reset-second-step`: turns the second step off by removing its file from `state_dir`, for someone who lost
/// both their phone or passkey and their recovery codes; the password alone signs in from then on, and the running
/// admin, which reads the file at every use, follows at once. Whether anything was on (a file it cannot read counts
/// as on).
pub fn reset(state_dir: &Path) -> Result<bool, String> {
    let file = state_dir.join(FILE);
    let _lock = lock(&file)?;
    let on = read(&file).map_or(true, |stored| stored.on());
    let removed = if_there(std::fs::remove_file(&file));
    if removed.map_err(|e| format!("{} cannot be removed: {e}", file.display()))?.is_none() {
        return Ok(false);
    }
    // The removal itself survives a power loss.
    std::fs::File::open(state_dir)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| format!("{}: {e}", state_dir.display()))?;
    Ok(on)
}

/// The second step as the server knows it at one moment: the file's contents, and what is pending in memory.
pub struct SecondStep {
    file: PathBuf,
    /// Sign-ins waiting for their second step, by the SHA-256 of their token.
    steps: HashMap<[u8; 32], Pending>,
    /// The authenticator app being added, by one login: a secret that does nothing until a code confirms it.
    app_setup: Option<AppSetup>,
    /// The relying party, when the admin's origin can have passkeys.
    webauthn: Option<Webauthn>,
    /// Passkey sign-ins begun, by the SHA-256 of their token.
    sign_ins: HashMap<[u8; 32], (Instant, PasskeyAuthentication)>,
    /// The passkey being added, by one login.
    passkey_setup: Option<PasskeySetup>,
}

struct PasskeySetup {
    login: LoginKey,
    user: Uuid,
    issued: Instant,
    state: PasskeyRegistration,
}

struct Pending {
    issued: Instant,
    attempts: u32,
    /// The KEK the right password derived (see `keys`), which the login this step opens unlocks the saved profiles
    /// with.
    unlock: Option<Kek>,
}

struct AppSetup {
    login: LoginKey,
    secret: [u8; totp::SECRET_BYTES],
    issued: Instant,
}

/// How a pending step was answered.
#[derive(Debug, PartialEq, Eq)]
pub enum Answered {
    /// The step and the code are used: the login opens, unlocking the saved profiles with the KEK its password derived.
    Right(Option<Kek>),
    Wrong,
    /// Wrong, and the last code the step takes: it is gone, and the password is asked again.
    WrongLast,
}

/// What the person may answer a pending step with.
#[derive(Debug, PartialEq, Eq)]
pub struct Methods {
    pub app: bool,
    pub passkey: bool,
}

/// Drops what `pending` holds past its `lifetime`, and then its oldest if it still holds `max`: room for one more.
fn make_room<V>(pending: &mut HashMap<[u8; 32], V>, lifetime: Duration, max: usize, issued: impl Fn(&V) -> Instant) {
    pending.retain(|_, held| issued(held).elapsed() < lifetime);
    if pending.len() >= max
        && let Some(oldest) = pending.iter().min_by_key(|(_, held)| issued(held)).map(|(key, _)| *key)
    {
        pending.remove(&oldest);
    }
}

/// A storage failure, logged where it happened and answered as one.
fn unsaved(why: String) -> Refusal {
    tracing::error!(%why, "the second step could not be read or written");
    Refusal::Failed("the second step could not be read or saved on the server; its log says why")
}

impl SecondStep {
    /// The second step of `state_dir`, whose file must be readable whole if it is there.
    pub fn load(state_dir: &Path) -> Result<SecondStep, String> {
        let second = SecondStep {
            file: state_dir.join(FILE),
            steps: HashMap::new(),
            app_setup: None,
            webauthn: None,
            sign_ins: HashMap::new(),
            passkey_setup: None,
        };
        second.read()?;
        Ok(second)
    }

    /// Offers passkeys for `origin` (`ADMIN_ORIGIN`), unless its host is an IP address, which no RP ID can be.
    pub fn offer_passkeys(&mut self, origin: &str) {
        let Ok(url) = Url::parse(origin) else { return };
        let Some(rp_id) = url.domain() else { return };
        self.webauthn = WebauthnBuilder::new(rp_id, &url).and_then(|b| b.rp_name("easi.run").build()).ok();
    }

    fn read(&self) -> Result<Stored, String> {
        read(&self.file)
    }

    /// Takes the lock every change holds (see [`lock`]), and then reads what is on.
    fn locked(&self) -> Result<(std::fs::File, Stored), Refusal> {
        let lock = lock(&self.file).map_err(unsaved)?;
        Ok((lock, self.read().map_err(unsaved)?))
    }

    /// Writes a change of what is on; with nothing on any more, the recovery codes go too. A failure once the new
    /// contents are in place (syncing their directory) is a change made all the same, so it is logged and taken as
    /// written: the caller ends the other logins and shows what it issued.
    fn write(&self, stored: &mut Stored) -> Result<(), String> {
        if !stored.on() {
            stored.recovery.clear();
        }
        let bytes = serde_json::to_vec(stored).expect("the second step serializes");
        match crate::private::replace(&self.file, &bytes) {
            Ok(()) => Ok(()),
            Err(why) if crate::private::read(&self.file).is_ok_and(|found| found.as_deref() == Some(&bytes[..])) => {
                tracing::warn!(%why, "the second step was written, but its directory could not be synced");
                Ok(())
            }
            Err(why) => Err(why),
        }
    }

    /// Writes the use of a code or a passkey's counter, which a sign-in waits for: only a write synced whole counts,
    /// since one a power loss could undo would let the same code, or the same count, sign in again.
    fn write_durably(&self, stored: &Stored) -> Result<(), String> {
        crate::private::replace(&self.file, &serde_json::to_vec(stored).expect("the second step serializes"))
    }

    /// After a right password, which derived `unlock`: a pending step, holding it, and what the step may be answered
    /// with while the second step is on; none while it is off.
    pub fn after_password(&mut self, unlock: Option<Kek>) -> Result<Option<(String, Methods)>, Refusal> {
        let stored = self.read().map_err(unsaved)?;
        if !stored.on() {
            return Ok(None);
        }
        make_room(&mut self.steps, STEP_LIFETIME, MAX_STEPS, |pending| pending.issued);
        let token = crate::auth::random_token();
        self.steps.insert(sha256(&token), Pending { issued: Instant::now(), attempts: 0, unlock });
        Ok(Some((token, Methods { app: stored.app.is_some(), passkey: self.passkey_sign_in_with(&stored) })))
    }

    /// Answers the pending step `step` with `code`: six digits from the authenticator app, or a recovery code. Right
    /// uses the step and the code up; a wrong one the caller counts toward the backoff. A step that is not pending, or
    /// no longer, is [`Refusal::Again`].
    pub fn answer(&mut self, step: &str, code: &str) -> Result<Answered, Refusal> {
        self.answer_at(step, code, unix_now())
    }

    fn answer_at(&mut self, step: &str, code: &str, now: u64) -> Result<Answered, Refusal> {
        let key = sha256(step);
        if self.steps.get(&key).is_none_or(|pending| pending.issued.elapsed() >= STEP_LIFETIME) {
            self.steps.remove(&key);
            return Err(Refusal::Again);
        }
        let (_lock, mut stored) = self.locked()?;
        let right = match &mut stored.app {
            Some(app) if code.len() == 6 => match totp::accept(&app.secret, code, now, Some(app.last_step)) {
                Some(step) => {
                    app.last_step = step;
                    true
                }
                None => false,
            },
            _ => stored.use_recovery_code(code),
        };
        if !right {
            let pending = self.steps.get_mut(&key).expect("checked above");
            pending.attempts += 1;
            if pending.attempts >= STEP_ATTEMPTS {
                self.steps.remove(&key);
                return Ok(Answered::WrongLast);
            }
            return Ok(Answered::Wrong);
        }
        // Written before the login opens: a code whose use is not on disk could be used again after a restart.
        self.write_durably(&stored).map_err(unsaved)?;
        let pending = self.steps.remove(&key).expect("checked above");
        Ok(Answered::Right(pending.unlock))
    }

    /// Forgets every pending step and passkey sign-in: the logins they would open ended with the others.
    pub fn forget_pending(&mut self) {
        self.steps.clear();
        self.sign_ins.clear();
    }

    /// What is on, for the Security page: never a secret.
    pub fn status(&self) -> Result<Value, Refusal> {
        let stored = self.read().map_err(unsaved)?;
        let passkeys: Vec<Value> = stored.passkeys.iter().map(|p| json!({"id": p.id, "created": p.created})).collect();
        Ok(json!({
            "app": stored.app.is_some(),
            "recovery_codes": stored.recovery.len(),
            "passkeys": passkeys,
            "passkeys_possible": self.webauthn.is_some(),
        }))
    }

    fn passkey_sign_in_with(&self, stored: &Stored) -> bool {
        self.webauthn.is_some() && !stored.passkeys.is_empty()
    }

    /// Whether the login screen offers a passkey: one is added, and this origin can have them. A file that cannot be
    /// read offers none (and refuses every sign-in besides).
    pub fn passkey_sign_in(&self) -> bool {
        self.read().is_ok_and(|stored| self.passkey_sign_in_with(&stored))
    }

    /// Begins a passkey sign-in: its token, and the options for the browser's `navigator.credentials.get`, which
    /// name the passkeys added and require user verification.
    pub fn begin_passkey_sign_in(&mut self) -> Result<(String, Value), Refusal> {
        let stored = self.read().map_err(unsaved)?;
        let webauthn = self.webauthn.as_ref().filter(|_| !stored.passkeys.is_empty()).ok_or(Refusal::Forbidden)?;
        let passkeys: Vec<Passkey> = stored.passkeys.iter().map(|p| p.passkey.clone()).collect();
        let (options, state) = webauthn.start_passkey_authentication(&passkeys).map_err(|e| {
            tracing::error!(why = %e, "a passkey sign-in could not begin");
            Refusal::Failed("the passkey sign-in could not begin")
        })?;
        make_room(&mut self.sign_ins, CEREMONY_LIFETIME, MAX_CEREMONIES, |(issued, _)| *issued);
        let token = crate::auth::random_token();
        self.sign_ins.insert(sha256(&token), (Instant::now(), state));
        Ok((token, serde_json::to_value(options).expect("the options serialize")))
    }

    /// Finishes the passkey sign-in `ceremony` with what the browser answered: true when one of the passkeys still
    /// added signed its challenge with user verification, false otherwise (the caller counts it toward the backoff).
    /// The ceremony is used either way; one that is not pending, or no longer, is [`Refusal::Again`].
    pub fn finish_passkey_sign_in(
        &mut self,
        ceremony: &str,
        credential: &PublicKeyCredential,
    ) -> Result<bool, Refusal> {
        let (issued, state) = self.sign_ins.remove(&sha256(ceremony)).ok_or(Refusal::Again)?;
        if issued.elapsed() >= CEREMONY_LIFETIME {
            return Err(Refusal::Again);
        }
        let webauthn = self.webauthn.as_ref().ok_or(Refusal::Forbidden)?;
        let result = match webauthn.finish_passkey_authentication(credential, &state) {
            Ok(result) => result,
            Err(e) => {
                tracing::info!(why = %e, "a passkey sign-in was refused");
                return Ok(false);
            }
        };
        // Removed (or reset) since the ceremony began: it signs in no more.
        let (_lock, mut stored) = self.locked()?;
        let Some(signed) = stored.passkeys.iter_mut().find(|p| p.passkey.cred_id() == result.cred_id()) else {
            return Ok(false);
        };
        // A device that keeps a signature counter must move it past the one stored now, not only past the one its
        // ceremony began with: two ceremonies begun together cannot both take the same count, as a copied key would.
        let stored_counter = counter(&signed.passkey);
        if (result.counter() > 0 || stored_counter > 0) && result.counter() <= stored_counter {
            tracing::warn!("a passkey's signature counter did not move on; it may have been copied");
            return Ok(false);
        }
        if signed.passkey.update_credential(&result) == Some(true) {
            self.write_durably(&stored).map_err(unsaved)?;
        }
        Ok(true)
    }

    /// Begins adding a passkey for `login` (the caller checked the current password): the options for the browser's
    /// `navigator.credentials.create`, which require user verification and leave out the passkeys added already.
    pub fn begin_passkey(&mut self, login: LoginKey) -> Result<Value, Refusal> {
        let webauthn = self.webauthn.as_ref().ok_or_else(|| {
            Refusal::BadRequest("passkeys need the admin at a domain name, such as localhost, not an IP address".into())
        })?;
        let stored = self.read().map_err(unsaved)?;
        if stored.passkeys.len() >= MAX_PASSKEYS {
            return Err(Refusal::BadRequest(format!("at most {MAX_PASSKEYS} passkeys; remove one first")));
        }
        let user = stored.user.unwrap_or_else(|| Uuid::from_bytes(rand::random()));
        let added = stored.passkeys.iter().map(|p| p.passkey.cred_id().clone()).collect();
        let (options, state) =
            webauthn.start_passkey_registration(user, "admin", "easi.run admin", Some(added)).map_err(|e| {
                tracing::error!(why = %e, "adding a passkey could not begin");
                Refusal::Failed("adding a passkey could not begin")
            })?;
        self.passkey_setup = Some(PasskeySetup { login, user, issued: Instant::now(), state });
        Ok(serde_json::to_value(options).expect("the options serialize"))
    }

    /// Adds the passkey `login` began with what the browser answered. Returns the new recovery codes when the second
    /// step was off until now.
    pub fn finish_passkey(
        &mut self,
        login: LoginKey,
        credential: &RegisterPublicKeyCredential,
    ) -> Result<Option<Vec<String>>, Refusal> {
        let setup = self
            .passkey_setup
            .take_if(|s| s.login == login)
            .filter(|s| s.issued.elapsed() < CEREMONY_LIFETIME)
            .ok_or_else(|| Refusal::BadRequest("that took too long; add the passkey again".into()))?;
        let webauthn = self.webauthn.as_ref().ok_or(Refusal::Forbidden)?;
        let passkey = webauthn.finish_passkey_registration(credential, &setup.state).map_err(|e| {
            tracing::info!(why = %e, "a passkey was refused");
            Refusal::BadRequest("this passkey could not be added; try again, or use another".into())
        })?;
        let (_lock, mut stored) = self.locked()?;
        if stored.passkeys.iter().any(|p| p.passkey.cred_id() == passkey.cred_id()) {
            return Err(Refusal::BadRequest("this passkey is added already".into()));
        }
        let was_on = stored.on();
        let id = crate::auth::random_token()[..12].to_owned();
        stored.passkeys.push(StoredPasskey { id, created: unix_now(), passkey });
        stored.user = Some(setup.user);
        let codes = (!was_on).then(|| stored.new_recovery_codes());
        self.write(&mut stored).map_err(unsaved)?;
        Ok(codes)
    }

    /// Removes the passkey `id` (the caller checked the current password); with nothing else on, the recovery codes
    /// go too.
    pub fn remove_passkey(&mut self, id: &str) -> Result<(), Refusal> {
        let (_lock, mut stored) = self.locked()?;
        let count = stored.passkeys.len();
        stored.passkeys.retain(|p| p.id != id);
        if stored.passkeys.len() == count {
            return Err(Refusal::BadRequest("no such passkey".into()));
        }
        self.write(&mut stored).map_err(unsaved)
    }

    /// Draws a new secret for the authenticator app, for `login` to confirm, and returns it as the app takes it: in
    /// base32, to type, and as the address its QR code carries. Nothing changes until it is confirmed.
    pub fn begin_app(&mut self, login: LoginKey) -> (String, String) {
        let secret = totp::new_secret();
        self.app_setup = Some(AppSetup { login, secret, issued: Instant::now() });
        (totp::base32(&secret), totp::uri(&secret))
    }

    /// Turns the authenticator app on with the secret `login` drew, once `code` shows the app has it: in place of the
    /// one on, if any, which takes the current password (`password_checked`). Returns the new recovery codes when the
    /// second step was off until now.
    pub fn confirm_app(
        &mut self,
        login: LoginKey,
        code: &str,
        password_checked: bool,
    ) -> Result<Option<Vec<String>>, Refusal> {
        self.confirm_app_at(login, code, password_checked, unix_now())
    }

    fn confirm_app_at(
        &mut self,
        login: LoginKey,
        code: &str,
        password_checked: bool,
        now: u64,
    ) -> Result<Option<Vec<String>>, Refusal> {
        let Some(setup) = self.app_setup.as_ref().filter(|s| s.login == login && s.issued.elapsed() < SETUP_LIFETIME)
        else {
            return Err(Refusal::BadRequest("that setup expired; start again".into()));
        };
        let (_lock, mut stored) = self.locked()?;
        if stored.app.is_some() && !password_checked {
            return Err(Refusal::BadRequest("replacing the authenticator app takes your current password".into()));
        }
        let Some(step) = totp::accept(&setup.secret, code, now, None) else {
            return Err(Refusal::BadRequest("that is not the code the app shows; try the next one".into()));
        };
        let was_on = stored.on();
        // The confirming code is used: it signs in nobody afterwards.
        stored.app = Some(AppFactor { secret: setup.secret, last_step: step });
        let codes = (!was_on).then(|| stored.new_recovery_codes());
        self.write(&mut stored).map_err(unsaved)?;
        self.app_setup = None;
        Ok(codes)
    }

    /// Turns the authenticator app off (the caller checked the current password); with nothing else on, the
    /// recovery codes go too.
    pub fn remove_app(&mut self) -> Result<(), Refusal> {
        let (_lock, mut stored) = self.locked()?;
        if stored.app.take().is_none() {
            return Err(Refusal::BadRequest("the authenticator app is off".into()));
        }
        self.write(&mut stored).map_err(unsaved)
    }

    /// New recovery codes in place of those left (the caller checked the current password), while the second step
    /// is on.
    pub fn new_recovery_codes(&mut self) -> Result<Vec<String>, Refusal> {
        let (_lock, mut stored) = self.locked()?;
        if !stored.on() {
            return Err(Refusal::BadRequest("turn the second step on first".into()));
        }
        let codes = stored.new_recovery_codes();
        self.write(&mut stored).map_err(unsaved)?;
        Ok(codes)
    }
}

impl SecondStep {
    /// The code the authenticator app shows `offset` steps from now: for the secret being added, or else the one on.
    #[cfg(test)]
    pub fn test_code(&self, offset: i64) -> String {
        let secret = match &self.app_setup {
            Some(setup) => setup.secret,
            None => self.read().unwrap().app.expect("the app is on").secret,
        };
        totp::code_at(&secret, unix_now().checked_add_signed(offset * 30).unwrap())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn login() -> LoginKey {
        crate::auth::key_of(&crate::auth::random_token())
    }

    /// The code the app shows `offset` steps from [`NOW`].
    fn code_for(secret: &[u8], offset: i64) -> String {
        totp::code_at(secret, NOW.checked_add_signed(offset * 30).unwrap())
    }

    fn app_code(second: &SecondStep, offset: i64) -> String {
        code_for(&second.read().unwrap().app.unwrap().secret, offset)
    }

    /// The second step with the authenticator app on, and its recovery codes.
    fn with_app(dir: &Path) -> (SecondStep, Vec<String>) {
        let mut second = SecondStep::load(dir).unwrap();
        let who = login();
        second.begin_app(who);
        let secret = second.app_setup.as_ref().unwrap().secret;
        let codes = second.confirm_app_at(who, &code_for(&secret, -1), false, NOW).unwrap().unwrap();
        (second, codes)
    }

    #[test]
    fn the_app_turns_on_only_with_a_code_from_its_secret_and_issues_recovery_codes_once() {
        let dir = crate::private::test_dir("second");
        let mut second = SecondStep::load(&dir).unwrap();
        assert_eq!(second.after_password(None), Ok(None), "off: the password alone signs in");
        let (who, other) = (login(), login());
        let (base32, uri) = second.begin_app(who);
        assert!(uri.contains(&format!("secret={base32}&")), "{uri}");
        assert!(second.read().unwrap().app.is_none(), "nothing is on until a code confirms it");
        let secret = second.app_setup.as_ref().unwrap().secret;
        let code = code_for(&secret, 0);
        assert!(second.confirm_app_at(other, &code, false, NOW).is_err(), "another login's setup");
        let wrong = if code == "000000" { "000001" } else { "000000" };
        assert!(second.confirm_app_at(who, wrong, false, NOW).is_err());
        let codes = second.confirm_app_at(who, &code, false, NOW).unwrap().expect("recovery codes");
        assert_eq!(codes.len(), RECOVERY_CODES);
        assert!(codes.iter().all(|c| c.len() == 19 && c.split('-').all(|group| group.len() == 4)), "{codes:?}");
        let stored = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(codes.iter().all(|c| !stored.contains(c.as_str())), "only their hashes are kept");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.join(FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        let status = second.status().unwrap();
        assert_eq!((&status["app"], &status["recovery_codes"]), (&json!(true), &json!(10)));
        // Replacing it takes the password, and keeps the recovery codes.
        second.begin_app(who);
        let secret = second.app_setup.as_ref().unwrap().secret;
        let code = code_for(&secret, 0);
        assert!(matches!(second.confirm_app_at(who, &code, false, NOW), Err(Refusal::BadRequest(_))));
        assert_eq!(second.confirm_app_at(who, &code, true, NOW), Ok(None));
        assert_eq!(second.read().unwrap().app.unwrap().secret, secret);
        assert_eq!(second.status().unwrap()["recovery_codes"], 10);
    }

    #[test]
    fn an_app_code_signs_in_once_and_never_after_a_newer_one() {
        let dir = crate::private::test_dir("second");
        let (mut second, _) = with_app(&dir);
        let pending = |second: &mut SecondStep| second.after_password(None).unwrap().expect("a pending step").0;
        let step = pending(&mut second);
        assert_eq!(
            second.answer_at(&step, &app_code(&second, -1), NOW),
            Ok(Answered::Wrong),
            "the confirming code is used"
        );
        assert_eq!(second.answer_at(&step, &app_code(&second, 0), NOW), Ok(Answered::Right(None)));
        assert_eq!(second.answer_at(&step, &app_code(&second, 1), NOW), Err(Refusal::Again), "the step is used");
        let step = pending(&mut second);
        assert_eq!(second.answer_at(&step, &app_code(&second, 0), NOW), Ok(Answered::Wrong), "a replay");
        // After a restart too: the step taken is on disk.
        let mut again = SecondStep::load(&dir).unwrap();
        let step = pending(&mut again);
        assert_eq!(again.answer_at(&step, &app_code(&again, 0), NOW), Ok(Answered::Wrong), "a replay after a restart");
        assert_eq!(again.answer_at(&step, &app_code(&again, 1), NOW), Ok(Answered::Right(None)), "the next one");
    }

    #[test]
    fn recovery_codes_sign_in_once_each_however_they_are_typed() {
        let dir = crate::private::test_dir("second");
        let (mut second, codes) = with_app(&dir);
        let step = second.after_password(None).unwrap().unwrap().0;
        let typed = format!(" {} ", codes[3].to_uppercase().replace('-', " "));
        assert_eq!(second.answer_at(&step, &typed, NOW), Ok(Answered::Right(None)));
        let step = second.after_password(None).unwrap().unwrap().0;
        assert_eq!(second.answer_at(&step, &codes[3], NOW), Ok(Answered::Wrong), "used");
        assert_eq!(second.answer_at(&step, "abcd-efgh-jkmn-pqrs", NOW), Ok(Answered::Wrong), "not one of them");
        assert_eq!(second.status().unwrap()["recovery_codes"], 9);
        assert_eq!(second.answer_at(&step, &codes[0], NOW), Ok(Answered::Right(None)));
        // New codes replace the ones left.
        let fresh = second.new_recovery_codes().unwrap();
        let step = second.after_password(None).unwrap().unwrap().0;
        assert_eq!(second.answer_at(&step, &codes[1], NOW), Ok(Answered::Wrong), "an old one");
        assert_eq!(second.answer_at(&step, &fresh[1], NOW), Ok(Answered::Right(None)));
    }

    #[test]
    fn a_pending_step_expires_and_takes_five_codes_at_most() {
        let dir = crate::private::test_dir("second");
        let (mut second, codes) = with_app(&dir);
        let step = second.after_password(None).unwrap().unwrap().0;
        second.steps.get_mut(&sha256(&step)).unwrap().issued -= STEP_LIFETIME;
        assert_eq!(second.answer_at(&step, &codes[0], NOW), Err(Refusal::Again), "expired");
        assert_eq!(second.answer_at("made up", &codes[0], NOW), Err(Refusal::Again));
        let step = second.after_password(None).unwrap().unwrap().0;
        for attempt in 1..=STEP_ATTEMPTS {
            let expected = if attempt == STEP_ATTEMPTS { Answered::WrongLast } else { Answered::Wrong };
            assert_eq!(second.answer_at(&step, "000000-nope", NOW), Ok(expected));
        }
        assert_eq!(second.answer_at(&step, &codes[0], NOW), Err(Refusal::Again), "dropped after five");
        for _ in 0..MAX_STEPS + 3 {
            second.after_password(None).unwrap();
        }
        assert_eq!(second.steps.len(), MAX_STEPS, "bounded");
        second.forget_pending();
        assert!(second.steps.is_empty());
    }

    #[test]
    fn a_file_it_cannot_read_never_turns_the_second_step_off() {
        let dir = crate::private::test_dir("second");
        let (mut second, codes) = with_app(&dir);
        let step = second.after_password(None).unwrap().unwrap().0;
        crate::private::replace(&dir.join(FILE), b"{\"app\": null, \"extra\": 1}").unwrap();
        assert!(matches!(second.after_password(None), Err(Refusal::Failed(_))));
        assert!(matches!(second.answer_at(&step, &codes[0], NOW), Err(Refusal::Failed(_))));
        assert!(SecondStep::load(&dir).err().unwrap().contains("reset-second-step"), "and stops the start");
        // A code whose use cannot be written signs nobody in, and stays good: a directory where the temporary file
        // goes stands for a disk that fails the write.
        std::fs::remove_file(dir.join(FILE)).unwrap();
        let (mut second, codes) = with_app(&dir);
        let step = second.after_password(None).unwrap().unwrap().0;
        std::fs::create_dir(dir.join(format!("{FILE}.tmp"))).unwrap();
        assert!(matches!(second.answer_at(&step, &codes[0], NOW), Err(Refusal::Failed(_))));
        std::fs::remove_dir(dir.join(format!("{FILE}.tmp"))).unwrap();
        assert_eq!(
            second.answer_at(&step, &codes[0], NOW),
            Ok(Answered::Right(None)),
            "the same code, once it can be written"
        );
    }

    #[test]
    fn turning_the_app_on_when_that_cannot_be_written_turns_nothing_on() {
        let dir = crate::private::test_dir("second");
        let mut second = SecondStep::load(&dir).unwrap();
        let who = login();
        second.begin_app(who);
        let code = code_for(&second.app_setup.as_ref().unwrap().secret, 0);
        // A directory where the temporary file goes stands for a disk that fails the write.
        std::fs::create_dir(dir.join(format!("{FILE}.tmp"))).unwrap();
        assert!(matches!(second.confirm_app_at(who, &code, false, NOW), Err(Refusal::Failed(_))), "no codes shown");
        assert_eq!(second.after_password(None), Ok(None), "still off: the password alone signs in");
        std::fs::remove_dir(dir.join(format!("{FILE}.tmp"))).unwrap();
        let codes = second.confirm_app_at(who, &code, false, NOW).unwrap();
        assert_eq!(codes.map(|codes| codes.len()), Some(RECOVERY_CODES), "the same code, once it can be written");
    }

    #[test]
    fn removing_the_app_turns_the_second_step_off_and_the_recovery_codes_go_with_it() {
        let dir = crate::private::test_dir("second");
        let (mut second, _) = with_app(&dir);
        second.remove_app().unwrap();
        assert_eq!(second.after_password(None), Ok(None));
        let status = second.status().unwrap();
        assert_eq!((&status["app"], &status["recovery_codes"]), (&json!(false), &json!(0)));
        assert!(second.remove_app().is_err(), "off already");
        assert!(second.new_recovery_codes().is_err());
    }

    /// A software authenticator holding one P-256 passkey, which answers WebAuthn's ceremonies as a browser passes
    /// them on: `none` attestation, user presence, and user verification unless `verified` is unset; its signature
    /// counter moves on at each sign-in unless `counts` is unset (a synced passkey's stays at zero).
    pub(crate) struct SoftKey {
        key: ring::signature::EcdsaKeyPair,
        id: Vec<u8>,
        counter: u32,
        counts: bool,
        origin: String,
        verified: bool,
    }

    const ORIGIN: &str = "https://localhost:9443";

    fn b64(bytes: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    impl SoftKey {
        pub(crate) fn new() -> SoftKey {
            use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair};
            let rng = ring::rand::SystemRandom::new();
            let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
            let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
            let id = rand::random::<[u8; 16]>().to_vec();
            SoftKey { key, id, counter: 0, counts: true, origin: ORIGIN.into(), verified: true }
        }

        fn client_data(&self, kind: &str, options: &Value) -> Vec<u8> {
            let challenge = options["publicKey"]["challenge"].as_str().unwrap();
            json!({"type": kind, "challenge": challenge, "origin": self.origin, "crossOrigin": false})
                .to_string()
                .into_bytes()
        }

        /// The authenticator data: the RP ID's hash, the flags, and the signature counter, then `attested` if any.
        fn authenticator_data(&self, rp_id: &str, attested: Option<&[u8]>) -> Vec<u8> {
            // User present, user verified (unless not), attested credential data included.
            let flags = 0x01 | if self.verified { 0x04 } else { 0 } | if attested.is_some() { 0x40 } else { 0 };
            let mut data = Sha256::digest(rp_id.as_bytes()).to_vec();
            data.push(flags);
            data.extend_from_slice(&self.counter.to_be_bytes());
            data.extend_from_slice(attested.unwrap_or_default());
            data
        }

        /// What `navigator.credentials.create` answers for `options`.
        fn register(&self, options: &Value) -> RegisterPublicKeyCredential {
            serde_json::from_value(self.registration(options)).unwrap()
        }

        /// [`SoftKey::register`]'s answer as the admin's page sends it, in JSON.
        pub(crate) fn registration(&self, options: &Value) -> Value {
            use ring::signature::KeyPair;
            let rp_id = options["publicKey"]["rp"]["id"].as_str().unwrap();
            let point = self.key.public_key().as_ref();
            // The COSE key, in CBOR: {1: 2 (EC2), 3: -7 (ES256), -1: 1 (P-256), -2: x, -3: y}.
            let mut cose = vec![0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
            cose.extend_from_slice(&point[1..33]);
            cose.extend_from_slice(&[0x22, 0x58, 0x20]);
            cose.extend_from_slice(&point[33..65]);
            let mut attested = vec![0u8; 16]; // an AAGUID of zeros
            attested.extend_from_slice(&(self.id.len() as u16).to_be_bytes());
            attested.extend_from_slice(&self.id);
            attested.extend_from_slice(&cose);
            let data = self.authenticator_data(rp_id, Some(&attested));
            // {"fmt": "none", "attStmt": {}, "authData": data}, in CBOR.
            let mut object = vec![0xa3, 0x63];
            object.extend_from_slice(b"fmt");
            object.push(0x64);
            object.extend_from_slice(b"none");
            object.push(0x67);
            object.extend_from_slice(b"attStmt");
            object.extend_from_slice(&[0xa0, 0x68]);
            object.extend_from_slice(b"authData");
            object.extend_from_slice(&[0x58, data.len() as u8]);
            object.extend_from_slice(&data);
            json!({
                "id": b64(&self.id),
                "rawId": b64(&self.id),
                "type": "public-key",
                "response": {"attestationObject": b64(&object), "clientDataJSON": b64(&self.client_data("webauthn.create", options))},
            })
        }

        /// What `navigator.credentials.get` answers for `options`, its counter one up if it counts.
        fn sign(&mut self, options: &Value) -> PublicKeyCredential {
            serde_json::from_value(self.assertion(options)).unwrap()
        }

        /// [`SoftKey::sign`]'s answer as the admin's page sends it, in JSON.
        pub(crate) fn assertion(&mut self, options: &Value) -> Value {
            self.counter += u32::from(self.counts);
            let rp_id = options["publicKey"]["rpId"].as_str().unwrap();
            let data = self.authenticator_data(rp_id, None);
            let client_data = self.client_data("webauthn.get", options);
            let mut signed = data.clone();
            signed.extend_from_slice(&Sha256::digest(&client_data));
            let signature = self.key.sign(&ring::rand::SystemRandom::new(), &signed).unwrap();
            json!({
                "id": b64(&self.id),
                "rawId": b64(&self.id),
                "type": "public-key",
                "response": {
                    "authenticatorData": b64(&data),
                    "clientDataJSON": b64(&client_data),
                    "signature": b64(signature.as_ref()),
                    "userHandle": null,
                },
            })
        }
    }

    /// The second step at [`ORIGIN`], with `key` added as a passkey for `who`, and the recovery codes it issued.
    fn with_passkey(dir: &Path, key: &SoftKey, who: LoginKey) -> (SecondStep, Option<Vec<String>>) {
        let mut second = SecondStep::load(dir).unwrap();
        second.offer_passkeys(ORIGIN);
        let options = second.begin_passkey(who).unwrap();
        let codes = second.finish_passkey(who, &key.register(&options)).unwrap();
        (second, codes)
    }

    #[test]
    fn reset_removes_the_file_whatever_it_holds() {
        let dir = crate::private::test_dir("second");
        assert_eq!(reset(&dir), Ok(false), "nothing was on");
        let (mut second, codes) = with_app(&dir);
        let step = second.after_password(None).unwrap().unwrap().0;
        assert_eq!(reset(&dir), Ok(true));
        assert_eq!(second.after_password(None), Ok(None), "the running admin reads it again: off at once");
        assert_eq!(second.answer_at(&step, &codes[0], NOW), Ok(Answered::Wrong), "and a recovery code is gone with it");
        crate::private::replace(&dir.join(FILE), b"not json").unwrap();
        assert_eq!(reset(&dir), Ok(true), "a file it cannot read counts as on");
        assert!(!dir.join(FILE).exists());
        crate::private::replace(&dir.join(FILE), b"{}").unwrap();
        assert_eq!(reset(&dir), Ok(false), "a file with nothing on");
    }

    #[test]
    fn a_passkey_is_added_with_user_verification_and_turns_the_second_step_on() {
        let dir = crate::private::test_dir("second");
        let (key, who) = (SoftKey::new(), login());
        let (mut second, codes) = with_passkey(&dir, &key, who);
        assert_eq!(codes.map(|c| c.len()), Some(RECOVERY_CODES), "the first factor issues the recovery codes");
        let status = second.status().unwrap();
        assert_eq!((status["app"].clone(), status["passkeys"].as_array().unwrap().len()), (json!(false), 1));
        assert_eq!(second.after_password(None).unwrap().unwrap().1, Methods { app: false, passkey: true });
        assert!(second.passkey_sign_in());
        // The options require user verification and leave out the passkey added already.
        let options = second.begin_passkey(who).unwrap();
        assert_eq!(options["publicKey"]["authenticatorSelection"]["userVerification"], "required");
        assert_eq!(options["publicKey"]["rp"]["id"], "localhost");
        assert_eq!(options["publicKey"]["excludeCredentials"][0]["id"], b64(&key.id));
        assert!(second.finish_passkey(who, &key.register(&options)).is_err(), "the same one twice");
        // Without user verification, from another origin, or for another login, none is added.
        let mut unverified = SoftKey::new();
        unverified.verified = false;
        let options = second.begin_passkey(who).unwrap();
        assert!(second.finish_passkey(who, &unverified.register(&options)).is_err());
        let mut elsewhere = SoftKey::new();
        elsewhere.origin = "https://localhost:9444".into();
        let options = second.begin_passkey(who).unwrap();
        assert!(second.finish_passkey(who, &elsewhere.register(&options)).is_err());
        let options = second.begin_passkey(who).unwrap();
        assert!(second.finish_passkey(login(), &SoftKey::new().register(&options)).is_err());
        assert_eq!(second.status().unwrap()["passkeys"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_passkey_signs_in_once_per_ceremony_with_its_counter_moving_on() {
        let dir = crate::private::test_dir("second");
        let (mut key, who) = (SoftKey::new(), login());
        let (mut second, _) = with_passkey(&dir, &key, who);
        let (ceremony, options) = second.begin_passkey_sign_in().unwrap();
        assert_eq!(options["publicKey"]["userVerification"], "required");
        let signed = key.sign(&options);
        assert_eq!(second.finish_passkey_sign_in(&ceremony, &signed), Ok(true));
        assert_eq!(second.finish_passkey_sign_in(&ceremony, &signed), Err(Refusal::Again), "the ceremony is used");
        // The same answer for a new ceremony signs another challenge: refused.
        let (ceremony, _) = second.begin_passkey_sign_in().unwrap();
        assert_eq!(second.finish_passkey_sign_in(&ceremony, &signed), Ok(false), "a replay");
        // A counter that went back shows a copied key: refused, the counter on disk being the one it reported.
        let (ceremony, options) = second.begin_passkey_sign_in().unwrap();
        key.counter -= 1;
        assert_eq!(second.finish_passkey_sign_in(&ceremony, &key.sign(&options)), Ok(false));
        let (ceremony, options) = second.begin_passkey_sign_in().unwrap();
        key.counter += 5;
        assert_eq!(second.finish_passkey_sign_in(&ceremony, &key.sign(&options)), Ok(true));
        // Two ceremonies begun together: the counter one took, the other cannot take again.
        let ((first, options), (other, also)) =
            (second.begin_passkey_sign_in().unwrap(), second.begin_passkey_sign_in().unwrap());
        assert_eq!(second.finish_passkey_sign_in(&first, &key.sign(&options)), Ok(true));
        key.counter -= 1;
        assert_eq!(second.finish_passkey_sign_in(&other, &key.sign(&also)), Ok(false), "the same count twice");
        // Without user verification, refused.
        let (ceremony, options) = second.begin_passkey_sign_in().unwrap();
        key.verified = false;
        assert_eq!(second.finish_passkey_sign_in(&ceremony, &key.sign(&options)), Ok(false));
        key.verified = true;
        let (ceremony, options) = second.begin_passkey_sign_in().unwrap();
        for (issued, _) in second.sign_ins.values_mut() {
            *issued -= CEREMONY_LIFETIME;
        }
        assert_eq!(second.finish_passkey_sign_in(&ceremony, &key.sign(&options)), Err(Refusal::Again), "expired");
    }

    #[test]
    fn a_passkey_that_keeps_no_counter_signs_in_every_time() {
        // A synced passkey (a phone's or a password manager's) reports zero at every sign-in.
        let dir = crate::private::test_dir("second");
        let (mut key, who) = (SoftKey { counts: false, ..SoftKey::new() }, login());
        let (mut second, _) = with_passkey(&dir, &key, who);
        for _ in 0..2 {
            let (ceremony, options) = second.begin_passkey_sign_in().unwrap();
            assert_eq!(second.finish_passkey_sign_in(&ceremony, &key.sign(&options)), Ok(true));
        }
    }

    #[test]
    fn a_removed_passkey_signs_in_no_more_even_in_a_ceremony_begun_before() {
        let dir = crate::private::test_dir("second");
        let (mut key, who) = (SoftKey::new(), login());
        let (mut second, codes) = with_passkey(&dir, &key, who);
        let (ceremony, options) = second.begin_passkey_sign_in().unwrap();
        let id = second.status().unwrap()["passkeys"][0]["id"].as_str().unwrap().to_owned();
        assert!(second.remove_passkey("not one").is_err());
        second.remove_passkey(&id).unwrap();
        assert_eq!(second.finish_passkey_sign_in(&ceremony, &key.sign(&options)), Ok(false));
        assert_eq!(second.after_password(None), Ok(None), "nothing else was on: off");
        assert_eq!(second.status().unwrap()["recovery_codes"], 0, "the recovery codes went with it");
        assert!(codes.is_some());
        assert_eq!(second.begin_passkey_sign_in().err(), Some(Refusal::Forbidden), "none to sign in with");
    }

    #[test]
    fn an_admin_at_an_ip_address_offers_no_passkeys() {
        let dir = crate::private::test_dir("second");
        let mut second = SecondStep::load(&dir).unwrap();
        second.offer_passkeys("https://127.0.0.1:9443");
        assert_eq!(second.status().unwrap()["passkeys_possible"], false);
        assert!(matches!(second.begin_passkey(login()), Err(Refusal::BadRequest(_))));
        second.offer_passkeys(ORIGIN);
        assert_eq!(second.status().unwrap()["passkeys_possible"], true);
    }
}
