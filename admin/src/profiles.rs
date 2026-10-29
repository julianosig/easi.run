//! Saved Chrome profiles: a session may start from one, so the logins and site data a person had in it carry over
//! from session to session. Never in the demo. People and agents know a profile as a saved session, and a session
//! started from one as that persistent session resumed.
//!
//! A profile has an id (16 hex digits), the name the administrator gives it, the agent tokens (and the local MCP
//! clients, `local`) allowed to start sessions from it, none at first, its data key wrapped by the KEK (see `keys`), and,
//! once a session from it ended, its sealed archive. `profiles.json` in the state directory holds all but the archives
//! (mode 0600, replaced atomically); `profiles/<id>.bin` holds each archive (mode 0600, written beside it and renamed
//! over it once whole). At most [`MAX_PROFILES`].
//!
//! The administrator may use every profile; a token, or the local clients, only one that allows it, and any other is
//! "no such saved session" to them, the answer a profile that does not exist gets. One session at a time uses a profile (a
//! second start is refused), so Chrome never has two writers. Without the KEK (since the admin started, the password was
//! neither set, nor used to log in, nor entered to unlock them; see `Auth`), the profiles are locked: creating one, or starting a session from one, is refused until
//! the administrator logs in with the password, or enters it once after a passkey sign-in; everything else works.
//!
//! At a session's start, before Chrome starts, the admin opens the profile's archive chunk by chunk and streams what it
//! opens to the browser's `profile-restore` through the controller (`profile_restore`); a profile not saved yet sends
//! an empty archive. When the session is stopped (`service::stop`), `profile-save` closes Chrome and streams the profile
//! back (`profile_save`), which the admin seals as it arrives, at most `MAX_ARCHIVE` bytes, into a file it renames over
//! the archive only once the end arrived. The plaintext is in the session's tmpfs and, while it streams, in the memory of
//! the admin, the controller, and Docker's exec stream (whose daemon's memory may be swapped, see docs/security.md);
//! easi.run never writes it to a file. A save that fails keeps the archive from
//! before, as does a session that ends without one: one whose start failed, or that the admin's restart ended.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::http::StatusCode;
use base64::Engine;
use display_protocol::profile::{Frame, HEADER, MAX_ARCHIVE, MAX_DATA, failure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

use crate::app::Owner;
use crate::auth::unix_now;
use crate::coded::{Code, Coded};
use crate::keys::{self, DataKey, Kek, Opener, PREFIX, Sealer, UNIT, WRAPPED};
use crate::private::{Unreplaced, if_there};
use crate::tokens::{MAX_NAME, fresh_id, id_text, parse_id, valid_name};

/// Profiles at once.
pub const MAX_PROFILES: usize = 16;
const FILE: &str = "profiles.json";
const ARCHIVES: &str = "profiles";
/// How long restoring a profile, or receiving one to save, may take in all (a save waits up to 20 seconds for Chrome to
/// close).
const DEADLINE: Duration = Duration::from_secs(120);
/// How long a failed save waits for its unfinished archive's removal, which goes on without it past that, so one that
/// stalls never holds the session's end.
const CLEANUP: Duration = Duration::from_secs(5);
/// How the allowed list names the local MCP clients.
const LOCAL: &str = "local";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    id: String,
    name: String,
    allowed: Vec<String>,
    /// The wrapped data key, in base64.
    key: String,
    created: u64,
    used: Option<u64>,
    size: Option<u64>,
    /// The egress (by id) its last session left through; none when it left directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    egress: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stores {
    profiles: Vec<Stored>,
}

/// Who besides the administrator may use a profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Allowed {
    Token(u64),
    Local,
}

impl Allowed {
    fn parse(text: &str) -> Option<Allowed> {
        if text == LOCAL { Some(Allowed::Local) } else { parse_id(text).map(Allowed::Token) }
    }

    fn text(self) -> String {
        match self {
            Allowed::Token(id) => id_text(id),
            Allowed::Local => LOCAL.to_owned(),
        }
    }
}

struct Profile {
    id: String,
    name: String,
    allowed: Vec<Allowed>,
    wrapped: [u8; WRAPPED],
    created: u64,
    /// When a session from it last ended with it saved, and the archive's size then.
    used: Option<u64>,
    size: Option<u64>,
    /// Why its last save failed, since the admin started, until one succeeds.
    unsaved: Option<Coded>,
    /// The egress (by id) its last session left through, none when directly: a resume that names none leaves there
    /// again, so the sites it is logged in to see it where they saw it before.
    egress: Option<String>,
}

impl Profile {
    /// Whether `who` may use it: the administrator always, a token or the local clients when it allows them.
    fn usable_by(&self, who: Owner) -> bool {
        match who {
            Owner::Admin => true,
            Owner::Token(id) => self.allowed.contains(&Allowed::Token(id)),
            Owner::Local => self.allowed.contains(&Allowed::Local),
            Owner::Visitor(_) => false,
        }
    }
}

/// Why a profile operation was refused, or failed.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// No profile has that id, or none that this caller may use.
    NoSuch,
    /// Nothing derived the KEK since the admin started (see `Auth`).
    Locked,
    /// Its key was wrapped by another KEK: the password it was made under is gone.
    Unreadable,
    /// A session uses it.
    InUse,
    BadName,
    /// An allowed list naming something other than a token's id or `local`.
    BadAllowed,
    TooMany,
    Unsaved(String),
}

impl Refused {
    /// The status a refusal is answered with, and what it says (see `coded`).
    pub fn answer(&self) -> (StatusCode, Coded) {
        match self {
            Refused::NoSuch => (StatusCode::NOT_FOUND, Coded::new(Code::NoSavedSession, "no such saved session")),
            Refused::Locked => (
                StatusCode::LOCKED,
                Coded::new(
                    Code::SavedLocked,
                    "saved sessions are locked until the administrator logs in with the password, or enters it once \
                     to unlock them",
                ),
            ),
            Refused::Unreadable => (
                StatusCode::CONFLICT,
                Coded::new(
                    Code::SavedOtherPassword,
                    "this saved session was made under another password and cannot be opened; delete it and make a \
                     new one",
                ),
            ),
            Refused::InUse => (
                StatusCode::CONFLICT,
                Coded::new(
                    Code::SavedRunning,
                    "this saved session is already running; stop it first, or resume another",
                ),
            ),
            Refused::BadName => (
                StatusCode::BAD_REQUEST,
                Coded::new(Code::SavedName, format!("name it in 1 to {MAX_NAME} characters, unlike the others"))
                    .with("max", MAX_NAME),
            ),
            Refused::BadAllowed => (
                StatusCode::BAD_REQUEST,
                Coded::new(Code::BadRequest, "allow agent tokens by their ids, and the local MCP clients as local"),
            ),
            Refused::TooMany => (
                StatusCode::CONFLICT,
                Coded::new(Code::TooManySaved, format!("at most {MAX_PROFILES} saved sessions; delete one first"))
                    .with("max", MAX_PROFILES),
            ),
            Refused::Unsaved(why) => (StatusCode::INTERNAL_SERVER_ERROR, Coded::new(Code::ServerUnsaved, why.clone())),
        }
    }
}

/// A profile a starting or running session uses: its id and name, and its data key, to restore and save it with.
pub struct ProfileUse {
    pub id: String,
    pub name: String,
    key: DataKey,
    /// Whether it was saved before: then its archive must be there.
    saved: bool,
    /// Whether the restore went through, so the session's end may save it. Held across a save, so a second stop waits
    /// for the first one's save rather than removing the browser under it.
    state: tokio::sync::Mutex<Saving>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Saving {
    NotRestored,
    Restored,
    Ended,
}

pub struct Profiles {
    file: PathBuf,
    archives: PathBuf,
    profiles: Vec<Profile>,
    /// The session using each profile, by the profile's id.
    in_use: HashMap<String, String>,
}

impl Profiles {
    /// No profiles (the demo's), never written.
    pub fn empty(state_dir: &Path) -> Profiles {
        Profiles {
            file: state_dir.join(FILE),
            archives: state_dir.join(ARCHIVES),
            profiles: Vec::new(),
            in_use: HashMap::new(),
        }
    }

    /// The stored profiles, none when there is no file yet. The file is read as a private file (see
    /// [`crate::private::read`]); one that cannot be read whole is refused: starting
    /// without it would lose every profile's key. An archive without a profile (one deleted while the admin stopped
    /// midway, or a save cut short) is removed.
    pub fn load(state_dir: &Path) -> Result<Profiles, String> {
        let mut loaded = Profiles::empty(state_dir);
        let file = &loaded.file;
        let stored: Stores = match crate::private::read(file)? {
            Some(bytes) => crate::private::json(file, &bytes)?,
            None => Stores { profiles: Vec::new() },
        };
        let malformed = || format!("{}: a malformed profile entry", file.display());
        for s in stored.profiles {
            let wrapped = base64::engine::general_purpose::STANDARD.decode(&s.key).ok().and_then(|k| k.try_into().ok());
            let allowed: Option<Vec<Allowed>> = s.allowed.iter().map(|a| Allowed::parse(a)).collect();
            let fresh = parse_id(&s.id).is_some() && valid_name(&s.name) && loaded.find(&s.id).is_none();
            let (Some(wrapped), Some(allowed), true) = (wrapped, allowed, fresh) else { return Err(malformed()) };
            let (created, used, size, egress) = (s.created, s.used, s.size, s.egress);
            loaded.profiles.push(Profile {
                id: s.id,
                name: s.name,
                allowed,
                wrapped,
                created,
                used,
                size,
                unsaved: None,
                egress,
            });
        }
        if loaded.profiles.len() > MAX_PROFILES {
            return Err(format!("{}: more than {MAX_PROFILES} profiles", file.display()));
        }
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().mode(0o700).create(&loaded.archives) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("{}: {e}", loaded.archives.display())),
        }
        let entries = std::fs::read_dir(&loaded.archives).map_err(|e| format!("{}: {e}", loaded.archives.display()))?;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let kept = name.to_str().and_then(|n| n.strip_suffix(".bin")).is_some_and(|id| loaded.find(id).is_some());
            if !kept {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        Ok(loaded)
    }

    fn find(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// The profile `id`, if `who` may use it.
    fn usable(&mut self, who: Owner, id: &str) -> Result<&mut Profile, Refused> {
        self.profiles.iter_mut().find(|p| p.id == id && p.usable_by(who)).ok_or(Refused::NoSuch)
    }

    /// Writes every profile, now.
    fn save(&self) -> Result<(), Unreplaced> {
        let profiles = self
            .profiles
            .iter()
            .map(|p| Stored {
                id: p.id.clone(),
                name: p.name.clone(),
                allowed: p.allowed.iter().map(|a| a.text()).collect(),
                key: base64::engine::general_purpose::STANDARD.encode(p.wrapped),
                created: p.created,
                used: p.used,
                size: p.size,
                egress: p.egress.clone(),
            })
            .collect();
        let bytes = serde_json::to_vec_pretty(&Stores { profiles })
            .map_err(|e| Unreplaced { why: e.to_string(), in_place: false })?;
        crate::private::replace(&self.file, &bytes)
    }

    /// Writes every profile, or puts `undo` back when that fails. A failure once the new contents are in place (syncing
    /// their directory) is a change made all the same, so it is kept: false then ([`crate::private::installed`]).
    fn save_or(&mut self, undo: impl FnOnce(&mut Profiles)) -> Result<bool, Refused> {
        crate::private::installed(self.save()).map_err(|why| {
            undo(self);
            tracing::error!(%why, "the profiles could not be written");
            Refused::Unsaved("the saved sessions could not be written on the server; its log says why".into())
        })
    }

    /// Where the archive of `id` is.
    pub fn archive(&self, id: &str) -> PathBuf {
        self.archives.join(format!("{id}.bin"))
    }

    /// The profiles `who` may use: for the administrator everything about them, for a token or the local clients their
    /// id, name, and whether a session uses them (`running`).
    pub fn list(&self, who: Owner) -> Vec<Value> {
        let usable = self.profiles.iter().filter(|p| p.usable_by(who));
        if who != Owner::Admin {
            return usable
                .map(|p| json!({"id": p.id, "name": p.name, "running": self.in_use.contains_key(&p.id)}))
                .collect();
        }
        usable.map(|p| self.describe(p)).collect()
    }

    fn describe(&self, p: &Profile) -> Value {
        json!({
            "id": p.id,
            "name": p.name,
            "allowed": p.allowed.iter().map(|a| a.text()).collect::<Vec<_>>(),
            "created": p.created,
            "used": p.used,
            "size": p.size,
            "session": self.in_use.get(&p.id),
            "unsaved": p.unsaved,
            "egress": p.egress,
        })
    }

    /// The egress (by id) the profile `id`'s last session left through, if `who` may use it and it did not leave
    /// directly.
    pub fn last_egress(&mut self, who: Owner, id: &str) -> Option<String> {
        self.usable(who, id).ok()?.egress.clone()
    }

    /// Remembers that a session of the profile `id` left through `egress` (none: directly). Kept in memory even when it
    /// cannot be written (the next write of the profiles writes it), so a resume while the admin runs never goes back to
    /// an earlier route; that failure is logged.
    pub fn left_through(&mut self, id: &str, egress: Option<&str>) {
        let Some(at) = self.profiles.iter().position(|p| p.id == id) else { return };
        let before = std::mem::replace(&mut self.profiles[at].egress, egress.map(str::to_owned));
        if before.as_deref() != egress
            && let Err(why) = self.save()
        {
            tracing::error!(%why, profile = %id, "a saved session's egress could not be written; kept until the next write");
        }
    }

    /// A name for a profile: allowed, and no other profile's (ignoring case) but `except`'s.
    fn name_for(&self, name: &str, except: Option<&str>) -> Result<String, Refused> {
        let name = name.trim();
        let taken =
            self.profiles.iter().any(|p| Some(p.id.as_str()) != except && p.name.to_lowercase() == name.to_lowercase());
        if !valid_name(name) || taken {
            return Err(Refused::BadName);
        }
        Ok(name.to_owned())
    }

    /// A new, empty profile named `name`, its data key wrapped by `kek`: its listing.
    pub fn create(&mut self, name: &str, kek: Option<&Kek>) -> Result<Value, Refused> {
        let name = self.name_for(name, None)?;
        if self.profiles.len() >= MAX_PROFILES {
            return Err(Refused::TooMany);
        }
        let kek = kek.ok_or(Refused::Locked)?;
        let id = id_text(fresh_id(|id| self.find(&id_text(id)).is_some()));
        let wrapped = keys::wrap(kek, &id, &DataKey::random());
        let created = unix_now();
        let profile = Profile {
            id,
            name,
            allowed: Vec::new(),
            wrapped,
            created,
            used: None,
            size: None,
            unsaved: None,
            egress: None,
        };
        self.profiles.push(profile);
        self.save_or(|profiles| drop(profiles.profiles.pop()))?;
        Ok(self.describe(self.profiles.last().expect("just added")))
    }

    /// Renames the profile `id`, or sets who besides the administrator may use it (token ids, and `local`): its new
    /// listing. The caller checks the tokens exist.
    pub fn update(&mut self, id: &str, name: Option<&str>, allowed: Option<&[String]>) -> Result<Value, Refused> {
        let name = name.map(|name| self.name_for(name, Some(id))).transpose()?;
        let allowed = match allowed {
            Some(list) => {
                let parsed: Vec<Allowed> =
                    list.iter().map(|a| Allowed::parse(a)).collect::<Option<_>>().ok_or(Refused::BadAllowed)?;
                // Each once, in the order given: the list is then at most every token and `local`, and the file stays
                // within what the admin reads back (`private::MAX_FILE`).
                let mut once: Vec<Allowed> = Vec::new();
                for a in parsed {
                    if !once.contains(&a) {
                        once.push(a);
                    }
                }
                Some(once)
            }
            None => None,
        };
        let at = self.profiles.iter().position(|p| p.id == id).ok_or(Refused::NoSuch)?;
        let before = (self.profiles[at].name.clone(), self.profiles[at].allowed.clone());
        let profile = &mut self.profiles[at];
        if let Some(name) = name {
            profile.name = name;
        }
        if let Some(allowed) = allowed {
            profile.allowed = allowed;
        }
        self.save_or(|profiles| (profiles.profiles[at].name, profiles.profiles[at].allowed) = before)?;
        Ok(self.describe(&self.profiles[at]))
    }

    /// Deletes the profile `id` and its archive, unless a session uses it.
    pub fn delete(&mut self, id: &str) -> Result<(), Refused> {
        let at = self.profiles.iter().position(|p| p.id == id).ok_or(Refused::NoSuch)?;
        if self.in_use.contains_key(id) {
            return Err(Refused::InUse);
        }
        let profile = self.profiles.remove(at);
        let archive = self.archive(id);
        // Written durably first: an archive left (by a failed removal, or kept while the profiles' removal may still
        // be undone) has no profile once that is, and the next start removes it.
        if self.save_or(|profiles| profiles.profiles.insert(at, profile))?
            && let Err(e) = if_there(std::fs::remove_file(&archive))
        {
            tracing::warn!(why = %e, "a deleted profile's archive could not be removed; the next start does");
        }
        Ok(())
    }

    /// Takes a revoked token off every profile.
    pub fn forget_token(&mut self, token: u64) {
        let gone = Allowed::Token(token);
        if !self.profiles.iter().any(|p| p.allowed.contains(&gone)) {
            return;
        }
        for profile in &mut self.profiles {
            profile.allowed.retain(|a| *a != gone);
        }
        if let Err(why) = self.save() {
            tracing::warn!(%why, "a revoked token stays listed on a profile; it can use none");
        }
    }

    /// Takes the profile `id` for `session`, started by `who`, with `kek` (the profiles' KEK, if a password unlocked
    /// them): its data key, if `who` may use it, it is unlocked and opens with that key, and no session uses it.
    pub fn reserve(&mut self, who: Owner, id: &str, session: &str, kek: Option<&Kek>) -> Result<ProfileUse, Refused> {
        let profile = self.usable(who, id)?;
        let kek = kek.ok_or(Refused::Locked)?;
        let key = keys::unwrap(kek, &profile.id, &profile.wrapped).ok_or(Refused::Unreadable)?;
        let (id, name, saved) = (profile.id.clone(), profile.name.clone(), profile.used.is_some());
        if self.in_use.contains_key(&id) {
            return Err(Refused::InUse);
        }
        self.in_use.insert(id.clone(), session.to_owned());
        Ok(ProfileUse { id, name, key, saved, state: tokio::sync::Mutex::new(Saving::NotRestored) })
    }

    /// Gives the profile `id` back, if `session` holds it.
    pub fn release(&mut self, id: &str, session: &str) {
        if self.in_use.get(id).is_some_and(|s| s == session) {
            self.in_use.remove(id);
        }
    }

    /// Records how the save at a session's end went: the archive's size, or why it was not saved.
    fn saved(&mut self, id: &str, outcome: Result<u64, Coded>) {
        let Some(profile) = self.profiles.iter_mut().find(|p| p.id == id) else { return };
        match outcome {
            Ok(size) => {
                (profile.used, profile.size, profile.unsaved) = (Some(unix_now()), Some(size), None);
                if let Err(why) = self.save() {
                    tracing::warn!(%why, "a profile's last use could not be written");
                }
            }
            Err(why) => profile.unsaved = Some(why),
        }
    }
}

/// Restores `profile` into the browser of `session` (see the module's notes), before Chrome starts.
pub async fn restore(app: &crate::app::App, session: &str, profile: &ProfileUse) -> Result<(), Coded> {
    let archive = app.profiles.lock().unwrap().archive(&profile.id);
    // What went wrong, which its own code does not tell (the controller's, the browser's), as one that failed.
    let failed = |why: String| Coded::new(Code::SavedNotRestored, why);
    let restored = async {
        let mut link = app.controller.profile_restore(session).await.map_err(failed)?;
        let sent = send(&archive, profile, link.get_mut()).await;
        let to = link.get_mut();
        let sent = match sent {
            Ok(()) => {
                let ended = to.write_all(&Frame::End.header()).await;
                ended.map_err(|e| failed(format!("resuming the saved session: {e}")))
            }
            Err(why) => {
                let body = failure(&why.why);
                let _ = to.write_all(&Frame::Failed(body.len()).header()).await;
                let _ = to.write_all(body).await;
                Err(why)
            }
        };
        // Read whatever happened here: a browser that refused the archive stops reading it, which fails the sending.
        match (sent, read_frame(&mut link).await) {
            (Ok(()), Ok((Frame::End, _))) => Ok(()),
            (Ok(()), Ok((Frame::Failed(_), why))) => {
                Err(failed(format!("the browser refused the saved session: {why}")))
            }
            (Ok(()), Ok((Frame::Data(_), _))) => Err(failed("the browser answered the restore with data".into())),
            (Err(ours), Ok((Frame::Failed(_), why))) => {
                let told = format!("{ours} (the browser: {why})");
                Err(ours.told(told))
            }
            (Ok(()), Err(why)) => Err(failed(why)),
            (Err(ours), _) => Err(ours),
        }
    };
    let restored = tokio::time::timeout(DEADLINE, restored).await;
    let restored = restored.unwrap_or_else(|_| Err(failed("resuming the saved session took too long".into())));
    if restored.is_ok() {
        *profile.state.lock().await = Saving::Restored;
    }
    restored
}

/// Opens the archive at `path` and sends what it holds to `to` as data frames: nothing, for a profile not saved yet. A
/// profile saved before whose archive is gone is refused, rather than started empty.
async fn send(path: &Path, profile: &ProfileUse, to: &mut (impl AsyncWrite + Unpin)) -> Result<(), Coded> {
    let unread = |e: std::io::Error| Coded::new(Code::SavedNotRestored, format!("the saved session's archive: {e}"));
    let unsent = |e: std::io::Error| Coded::new(Code::SavedNotRestored, format!("resuming the saved session: {e}"));
    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !profile.saved => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Coded::new(
                Code::SavedDataMissing,
                "the saved session's data is missing from the server; restore it, or delete the saved session",
            ));
        }
        Err(e) => return Err(unread(e)),
    };
    let damaged =
        || Coded::new(Code::SavedArchiveBroken, "the saved session's archive does not open: it was changed or cut");
    let length = file.metadata().await.map_err(unread)?.len();
    if length > keys::sealed_length(MAX_ARCHIVE) {
        return Err(damaged());
    }
    let units = keys::units(length).ok_or_else(damaged)?;
    let mut prefix = [0u8; PREFIX];
    file.read_exact(&mut prefix).await.map_err(unread)?;
    let mut opener = Some(Opener::new(&profile.key, &profile.id, &prefix));
    let mut sealed = vec![0u8; UNIT];
    for (i, &(_, length)) in units.iter().enumerate() {
        file.read_exact(&mut sealed[..length]).await.map_err(unread)?;
        let plain = if i + 1 == units.len() {
            opener.take().and_then(|o| o.open_last(&sealed[..length]))
        } else {
            opener.as_mut().and_then(|o| o.open(&sealed[..length]))
        };
        let plain = plain.ok_or_else(damaged)?;
        for piece in plain.chunks(MAX_DATA) {
            to.write_all(&Frame::Data(piece.len()).header()).await.map_err(unsent)?;
            to.write_all(piece).await.map_err(unsent)?;
        }
    }
    Ok(())
}

/// One frame from `from`, with its body if it is a failure's (as text).
async fn read_frame(from: &mut (impl AsyncRead + Unpin)) -> Result<(Frame, String), String> {
    let mut header = [0u8; HEADER];
    from.read_exact(&mut header).await.map_err(|_| "the browser ended before it answered".to_owned())?;
    let frame = Frame::parse(header)?;
    let mut why = String::new();
    if let Frame::Failed(length) = frame {
        let mut body = vec![0u8; length];
        from.read_exact(&mut body).await.map_err(|_| "the browser ended before it answered".to_owned())?;
        why = String::from_utf8_lossy(&body).into_owned();
    }
    Ok((frame, why))
}

/// Saves the profile of a session that is ending, once (see the module's notes): only one whose restore went through.
/// Whatever happens, the session goes on ending; a save that fails is logged and listed with the profile, which keeps
/// the archive from before.
pub async fn save_at_end(app: &crate::app::App, session: &str, profile: &ProfileUse) {
    let mut state = profile.state.lock().await;
    if *state != Saving::Restored {
        return;
    }
    *state = Saving::Ended;
    let saved = save(app, session, profile).await;
    match &saved {
        Ok(size) => tracing::info!(session, profile = %profile.id, size, "profile saved"),
        // Escaped (`Debug`): the browser's part of it is a page's to choose (a file name), and must not forge a log line.
        Err(why) => {
            tracing::warn!(session, profile = %profile.id, ?why, "the profile was not saved; it keeps its last save")
        }
    }
    let shown = saved.map_err(|why| {
        let told = format!("{why}; it keeps its last save");
        why.told(told)
    });
    app.profiles.lock().unwrap().saved(&profile.id, shown);
}

/// Receives the profile from the browser of `session`, sealed into a file of this save's own beside its archive (see
/// [`temporary`]), then renamed over it: the archive's size. Only the transfer is timed: one that takes longer than
/// [`DEADLINE`] fails. The rename and its directory's sync finish while the session still holds the profile, so what is
/// reported is what happened. A failed save removes its own file beside the archive, and no other.
async fn save(app: &crate::app::App, session: &str, profile: &ProfileUse) -> Result<u64, Coded> {
    let archive = app.profiles.lock().unwrap().archive(&profile.id);
    let temporary = temporary(&archive);
    let failed = |why: String| Coded::new(Code::SaveFailed, why);
    let received = async {
        let mut link = app.controller.profile_save(session).await.map_err(failed)?;
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .await
            .map_err(|e| failed(format!("{}: {e}", temporary.display())))?;
        receive(&mut link, profile, &mut file, MAX_ARCHIVE).await?;
        file.sync_all().await.map_err(|e| failed(format!("{}: {e}", temporary.display())))?;
        let size = file.metadata().await.map(|m| m.len());
        size.map_err(|e| failed(format!("{}: {e}", temporary.display())))
    };
    let received = tokio::time::timeout(DEADLINE, received).await;
    let saved = match received {
        Ok(Ok(size)) => tokio::fs::rename(&temporary, &archive)
            .await
            .map(|()| size)
            .map_err(|e| failed(format!("{}: {e}", archive.display()))),
        Ok(Err(why)) => Err(why),
        Err(_) => Err(failed("saving the session took too long".into())),
    };
    if saved.is_err() {
        let _ = tokio::time::timeout(CLEANUP, tokio::fs::remove_file(&temporary)).await;
        return saved;
    }
    let synced = tokio::task::spawn_blocking(move || crate::private::sync_parent(&archive)).await;
    if !matches!(synced, Ok(Ok(()))) {
        tracing::warn!("a saved profile's directory could not be synced");
    }
    saved
}

/// A file name of one save's own beside `archive`, so a removal that outlasts its save never reaches a later one's. One
/// a stop left is removed at the next start, as it names no archive (see [`Profiles::load`]).
fn temporary(archive: &Path) -> PathBuf {
    let mut name = archive.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{:016x}.tmp", rand::random::<u64>()));
    archive.with_file_name(name)
}

/// Seals the archive `from` streams into `to`, until its end frame: at most `max` bytes of it (`MAX_ARCHIVE`).
async fn receive(
    from: &mut (impl AsyncRead + Unpin),
    profile: &ProfileUse,
    to: &mut (impl AsyncWrite + Unpin),
    max: u64,
) -> Result<(), Coded> {
    let (mut sealer, prefix) = Sealer::new(&profile.key, &profile.id);
    let write = |e: std::io::Error| Coded::new(Code::SaveFailed, format!("writing the saved session's archive: {e}"));
    let ended = || Coded::new(Code::SaveFailed, "the browser ended before its session was saved");
    to.write_all(&prefix).await.map_err(write)?;
    let mut piece = Zeroizing::new(vec![0u8; MAX_DATA]);
    let mut sealed = Vec::with_capacity(2 * UNIT);
    let mut total = 0u64;
    loop {
        let (frame, why) = read_frame(from).await.map_err(|why| Coded::new(Code::SaveFailed, why))?;
        match frame {
            Frame::Data(length) => {
                total += length as u64;
                if total > max {
                    let why = format!("what the session keeps is larger than {} MiB, so it was not saved", max >> 20);
                    return Err(Coded::new(Code::SaveTooLarge, why).with("max", max >> 20));
                }
                from.read_exact(&mut piece[..length]).await.map_err(|_| ended())?;
                sealer.push(&piece[..length], &mut sealed);
                to.write_all(&sealed).await.map_err(write)?;
                sealed.clear();
            }
            Frame::End => break,
            Frame::Failed(_) => {
                return Err(Coded::new(Code::SaveFailed, format!("the browser could not save it: {why}")));
            }
        }
    }
    sealer.finish(&mut sealed);
    to.write_all(&sealed).await.map_err(write)?;
    to.flush().await.map_err(write)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kek() -> Kek {
        keys::derive("correct horse battery", &[7; keys::SALT])
    }

    #[test]
    fn a_profile_s_key_opens_only_with_the_password_it_was_made_under() {
        let dir = crate::private::test_dir("profiles");
        let mut profiles = Profiles::load(&dir).unwrap();
        assert_eq!(profiles.create("Personal", None), Err(Refused::Locked), "locked: nothing can wrap its key");
        let id = profiles.create("Personal", Some(&kek())).unwrap()["id"].as_str().unwrap().to_owned();
        // After a restart the profiles are as they were, and locked until a password unlocks them.
        let mut restarted = Profiles::load(&dir).unwrap();
        assert_eq!(restarted.reserve(Owner::Admin, &id, "s1", None).err(), Some(Refused::Locked));
        let other = keys::derive("another long password", &[7; keys::SALT]);
        assert_eq!(restarted.reserve(Owner::Admin, &id, "s1", Some(&other)).err(), Some(Refused::Unreadable));
        let used = restarted.reserve(Owner::Admin, &id, "s1", Some(&kek())).unwrap();
        assert_eq!((used.id.as_str(), used.name.as_str()), (id.as_str(), "Personal"));
    }

    #[test]
    fn a_token_uses_only_the_profiles_that_allow_it_and_learns_of_no_other() {
        let dir = crate::private::test_dir("profiles");
        let mut profiles = Profiles::load(&dir).unwrap();
        let id = profiles.create("Personal", Some(&kek())).unwrap()["id"].as_str().unwrap().to_owned();
        let (mine, other) = (Owner::Token(1), Owner::Token(2));
        for who in [mine, Owner::Local, Owner::Visitor(crate::auth::key_of("v"))] {
            assert_eq!(profiles.reserve(who, &id, "s1", Some(&kek())).err(), Some(Refused::NoSuch), "{who:?}");
            assert_eq!(profiles.reserve(who, &id, "s1", None).err(), Some(Refused::NoSuch), "locked or not");
            assert!(profiles.list(who).is_empty(), "{who:?} lists none");
        }
        // Repeated, apart or side by side: kept once each, in order.
        let allowed = [id_text(1), LOCAL.to_owned(), id_text(1), LOCAL.to_owned(), LOCAL.to_owned()];
        let listed = profiles.update(&id, None, Some(&allowed)).unwrap();
        assert_eq!(listed["allowed"], json!([id_text(1), LOCAL]));
        assert_eq!(profiles.reserve(other, &id, "s1", Some(&kek())).err(), Some(Refused::NoSuch));
        assert_eq!(profiles.reserve(mine, &id, "s1", None).err(), Some(Refused::Locked), "allowed, but locked");
        assert_eq!(profiles.list(mine), vec![json!({"id": id, "name": "Personal", "running": false})]);
        assert!(profiles.list(other).is_empty());
        let used = profiles.reserve(mine, &id, "s1", Some(&kek())).unwrap();
        profiles.release(&used.id, "s1");
        assert!(profiles.reserve(Owner::Local, &id, "s2", Some(&kek())).is_ok(), "the local clients when allowed");
        assert_eq!(profiles.reserve(mine, "0123456789abcdef", "s3", Some(&kek())).err(), Some(Refused::NoSuch));
        profiles.release(&id, "s2");
        profiles.forget_token(1);
        assert_eq!(profiles.reserve(mine, &id, "s4", Some(&kek())).err(), Some(Refused::NoSuch), "revoked: off it");
        assert_eq!(Profiles::load(&dir).unwrap().list(Owner::Admin)[0]["allowed"], json!([LOCAL]), "written so");
    }

    #[test]
    fn one_session_at_a_time_uses_a_profile() {
        let dir = crate::private::test_dir("profiles");
        let mut profiles = Profiles::load(&dir).unwrap();
        let id = profiles.create("Work", Some(&kek())).unwrap()["id"].as_str().unwrap().to_owned();
        let _used = profiles.reserve(Owner::Admin, &id, "s1", Some(&kek())).unwrap();
        assert_eq!(profiles.reserve(Owner::Admin, &id, "s2", Some(&kek())).err(), Some(Refused::InUse));
        assert_eq!(profiles.delete(&id), Err(Refused::InUse), "nor deleted while in use");
        assert_eq!(profiles.list(Owner::Admin)[0]["session"], "s1");
        profiles.release(&id, "s2");
        assert_eq!(profiles.reserve(Owner::Admin, &id, "s2", Some(&kek())).err(), Some(Refused::InUse), "not s2's");
        profiles.release(&id, "s1");
        assert!(profiles.reserve(Owner::Admin, &id, "s2", Some(&kek())).is_ok(), "free once its session gave it back");
    }

    #[test]
    fn names_are_bounded_and_distinct_and_the_count_too() {
        let dir = crate::private::test_dir("profiles");
        let mut profiles = Profiles::load(&dir).unwrap();
        for bad in ["", "   ", &"a".repeat(MAX_NAME + 1), "tab\there"] {
            assert_eq!(profiles.create(bad, Some(&kek())).err(), Some(Refused::BadName), "{bad:?}");
        }
        let id = profiles.create(" Personal ", Some(&kek())).unwrap()["id"].as_str().unwrap().to_owned();
        assert_eq!(profiles.list(Owner::Admin)[0]["name"], "Personal", "trimmed");
        assert_eq!(profiles.create("personal", Some(&kek())).err(), Some(Refused::BadName), "taken, ignoring case");
        assert!(profiles.update(&id, Some("PERSONAL"), None).is_ok(), "its own name, in another case");
        for n in 1..MAX_PROFILES {
            profiles.create(&format!("p{n}"), Some(&kek())).unwrap();
        }
        assert_eq!(profiles.create("one more", Some(&kek())).err(), Some(Refused::TooMany));
        assert_eq!(profiles.update(&id, None, Some(&["nobody".to_owned()])).err(), Some(Refused::BadAllowed));
    }

    #[test]
    fn deleting_a_profile_removes_its_archive_and_a_start_removes_what_no_profile_owns() {
        let dir = crate::private::test_dir("profiles");
        let mut profiles = Profiles::load(&dir).unwrap();
        let id = profiles.create("Personal", Some(&kek())).unwrap()["id"].as_str().unwrap().to_owned();
        std::fs::write(profiles.archive(&id), b"sealed").unwrap();
        std::fs::write(dir.join(ARCHIVES).join("fedcba9876543210.bin"), b"left over").unwrap();
        let cut_short = temporary(&profiles.archive(&id));
        assert_ne!(cut_short, temporary(&profiles.archive(&id)), "each save writes a file of its own");
        std::fs::write(cut_short, b"cut short").unwrap();
        let mut profiles = Profiles::load(&dir).unwrap();
        let left: Vec<_> = std::fs::read_dir(dir.join(ARCHIVES)).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from(format!("{id}.bin"))]);
        profiles.delete(&id).unwrap();
        assert!(!profiles.archive(&id).exists() && Profiles::load(&dir).unwrap().list(Owner::Admin).is_empty());
        assert_eq!(profiles.delete(&id), Err(Refused::NoSuch));
        crate::private::replace(&dir.join(FILE), b"{\"profiles\": [{\"id\": \"x\"}]}").unwrap();
        let refused = Profiles::load(&dir).err().expect("a file it cannot read whole stops the start");
        assert!(refused.contains("not the expected JSON"), "refused for its contents: {refused}");
    }

    /// The frames a launcher's `profile-save` sends for `archive`.
    fn frames(archive: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for piece in archive.chunks(MAX_DATA) {
            out.extend_from_slice(&Frame::Data(piece.len()).header());
            out.extend_from_slice(piece);
        }
        out.extend_from_slice(&Frame::End.header());
        out
    }

    fn used() -> ProfileUse {
        let dir = crate::private::test_dir("profiles");
        let mut profiles = Profiles::load(&dir).unwrap();
        let id = profiles.create("Personal", Some(&kek())).unwrap()["id"].as_str().unwrap().to_owned();
        profiles.reserve(Owner::Admin, &id, "s1", Some(&kek())).unwrap()
    }

    #[tokio::test]
    async fn a_saved_archive_is_sealed_and_restores_the_same_bytes() {
        let profile = used();
        let archive: Vec<u8> = (0..3 * MAX_DATA + 99).map(|i| (i % 253) as u8).collect();
        let mut sealed = Vec::new();
        receive(&mut &frames(&archive)[..], &profile, &mut sealed, MAX_ARCHIVE).await.unwrap();
        assert!(!sealed.windows(64).any(|w| archive.windows(64).next() == Some(w)), "nothing of it in the clear");
        let dir = crate::private::test_dir("profiles");
        let path = dir.join("p.bin");
        std::fs::write(&path, &sealed).unwrap();
        let mut sent = Vec::new();
        send(&path, &profile, &mut sent).await.unwrap();
        sent.extend_from_slice(&Frame::End.header());
        assert_eq!(sent, frames(&archive), "the same archive, framed as the launcher takes it");
        let mut nothing = Vec::new();
        send(&dir.join("never-saved.bin"), &profile, &mut nothing).await.unwrap();
        assert!(nothing.is_empty(), "a profile never saved sends an empty archive");
        let saved_before = ProfileUse { saved: true, ..used() };
        let why = send(&dir.join("never-saved.bin"), &saved_before, &mut nothing).await.unwrap_err();
        assert_eq!(
            why.code,
            Code::SavedDataMissing,
            "one saved before whose archive is gone is not started empty: {why}"
        );
        let sparse = std::fs::File::create(dir.join("sparse.bin")).unwrap();
        sparse.set_len(keys::sealed_length(MAX_ARCHIVE) + 1).unwrap();
        let why = send(&dir.join("sparse.bin"), &profile, &mut nothing).await.unwrap_err();
        assert_eq!(why.code, Code::SavedArchiveBroken, "longer than any archive: refused before it is read: {why}");
        let mut cut = sealed.clone();
        cut.truncate(sealed.len() - 1);
        std::fs::write(&path, &cut).unwrap();
        assert_eq!(send(&path, &profile, &mut Vec::new()).await.unwrap_err().code, Code::SavedArchiveBroken);
    }

    /// Data frames of zeros, without end.
    struct Endless(usize);

    impl AsyncRead for Endless {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            let (frame, at) = (HEADER + MAX_DATA, self.0 % (HEADER + MAX_DATA));
            let n = if at < HEADER {
                let n = buf.remaining().min(HEADER - at);
                buf.put_slice(&Frame::Data(MAX_DATA).header()[at..at + n]);
                n
            } else {
                let n = buf.remaining().min(frame - at).min(4096);
                buf.put_slice(&[0; 4096][..n]);
                n
            };
            self.0 += n;
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn a_save_past_the_bound_or_cut_or_failed_is_refused() {
        let profile = used();
        // Past the bound: refused once the frames carry more than it, before the rest arrives.
        let max = 3 << 20;
        let why = receive(&mut Endless(0), &profile, &mut tokio::io::sink(), max).await.unwrap_err();
        assert_eq!(why.json()["max"], 3, "{why}");
        assert!(why.why.contains("larger than 3 MiB") && why.code == Code::SaveTooLarge, "{why}");
        let exact = frames(&vec![1; max as usize]);
        assert!(receive(&mut &exact[..], &profile, &mut tokio::io::sink(), max).await.is_ok(), "the bound itself fits");
        let mut cut = frames(b"part of an archive");
        cut.truncate(cut.len() - HEADER);
        let why = receive(&mut &cut[..], &profile, &mut Vec::new(), MAX_ARCHIVE).await.unwrap_err();
        assert!(why.why.contains("ended before"), "{why}");
        let mut failed = cut.clone();
        failed.extend_from_slice(&Frame::Failed(5).header());
        failed.extend_from_slice(b"stuck");
        let why = receive(&mut &failed[..], &profile, &mut Vec::new(), MAX_ARCHIVE).await.unwrap_err();
        assert!(why.why.contains("stuck"), "{why}");
    }

    /// What a fake controller's sessions do with a profile: the archive each `profile_save` sends, in turn, whether a
    /// `profile_restore` is refused, and the bytes the last one received; and every op asked.
    #[derive(Default)]
    struct Browser {
        saves: std::collections::VecDeque<Vec<u8>>,
        refuse_restore: bool,
        restored: Vec<u8>,
        ops: Vec<String>,
    }

    /// A controller on a Unix socket in `dir` whose browsers take a restore whole and answer it (see
    /// [`Browser::refuse_restore`]), and send each save [`Browser::saves`] holds.
    fn fake_controller(dir: &Path) -> (crate::controller::Controller, std::sync::Arc<std::sync::Mutex<Browser>>) {
        let browser = std::sync::Arc::new(std::sync::Mutex::new(Browser::default()));
        let shared = browser.clone();
        let controller = crate::controller::fake(dir.join("controller.sock"), move |request, mut stream| {
            let op = request["op"].as_str().unwrap().to_owned();
            shared.lock().unwrap().ops.push(op.clone());
            let shared = shared.clone();
            async move {
                stream.get_mut().write_all(b"{\"ok\":true}\n").await.unwrap();
                if op == "profile_restore" {
                    let mut restored = Vec::new();
                    loop {
                        let mut header = [0u8; HEADER];
                        stream.read_exact(&mut header).await.unwrap();
                        let Frame::Data(length) = Frame::parse(header).unwrap() else { break };
                        let mut piece = vec![0u8; length];
                        stream.read_exact(&mut piece).await.unwrap();
                        restored.extend(piece);
                    }
                    let refused = {
                        let mut browser = shared.lock().unwrap();
                        browser.restored = restored;
                        browser.refuse_restore
                    };
                    let answer = if refused {
                        [&Frame::Failed(7).header()[..], b"no room"].concat()
                    } else {
                        Frame::End.header().to_vec()
                    };
                    stream.get_mut().write_all(&answer).await.unwrap();
                } else if op == "profile_save" {
                    let save = shared.lock().unwrap().saves.pop_front().unwrap();
                    stream.get_mut().write_all(&save).await.unwrap();
                }
            }
        });
        (controller, browser)
    }

    #[tokio::test]
    async fn a_session_saves_its_profile_only_once_restored_and_a_failed_save_keeps_the_last_one() {
        let socket = crate::private::test_dir("profiles-controller");
        let (controller, browser) = fake_controller(&socket);
        let app = crate::app::test_app_with(false, controller);
        let kek = kek();
        let id =
            app.profiles.lock().unwrap().create("Personal", Some(&kek)).unwrap()["id"].as_str().unwrap().to_owned();
        let reserve = |session: &str| app.profiles.lock().unwrap().reserve(Owner::Admin, &id, session, Some(&kek));
        let release = |session: &str| app.profiles.lock().unwrap().release(&id, session);
        let listed = || app.profiles.lock().unwrap().list(Owner::Admin)[0].clone();
        let saves = || browser.lock().unwrap().ops.iter().filter(|op| *op == "profile_save").count();
        let archive = app.profiles.lock().unwrap().archive(&id);
        // Restored, a session saves its profile at its end, once however often it is stopped.
        let logins = b"the person's logins ".repeat(5000);
        browser.lock().unwrap().saves.push_back(frames(&logins));
        let used = reserve("s1").unwrap();
        restore(&app, "s1", &used).await.unwrap();
        save_at_end(&app, "s1", &used).await;
        save_at_end(&app, "s1", &used).await;
        assert_eq!(saves(), 1);
        let sealed = std::fs::read(&archive).unwrap();
        assert_eq!((listed()["size"].clone(), listed()["unsaved"].clone()), (json!(sealed.len()), Value::Null));
        release("s1");
        // A session whose restore failed ends without a save: the empty profile it ran with replaces nothing.
        browser.lock().unwrap().refuse_restore = true;
        let unrestored = reserve("s2").unwrap();
        let refused = restore(&app, "s2", &unrestored).await.unwrap_err();
        assert!(refused.why.contains("no room") && refused.code == Code::SavedNotRestored, "{refused}");
        save_at_end(&app, "s2", &unrestored).await;
        assert_eq!((saves(), std::fs::read(&archive).unwrap()), (1, sealed.clone()), "not saved");
        browser.lock().unwrap().refuse_restore = false;
        release("s2");
        // A save the browser cuts short, or reports failed, keeps the last one whole, and says so.
        let mut cut = frames(b"part of a newer profile");
        cut.truncate(cut.len() - HEADER);
        let mut failed = cut.clone();
        failed.extend_from_slice(&Frame::Failed(5).header());
        failed.extend_from_slice(b"stuck");
        for (attempt, (save, why)) in
            [(cut, "ended before"), (failed, "could not save it: stuck")].into_iter().enumerate()
        {
            browser.lock().unwrap().saves.push_back(save);
            let used = reserve("s3").unwrap();
            restore(&app, "s3", &used).await.unwrap();
            save_at_end(&app, "s3", &used).await;
            assert_eq!(saves(), 2 + attempt, "the browser was asked for it");
            assert!(browser.lock().unwrap().saves.is_empty(), "and sent what it had");
            assert_eq!(std::fs::read(&archive).unwrap(), sealed, "the last save, byte for byte");
            let left = std::fs::read_dir(archive.parent().unwrap()).unwrap().count();
            assert_eq!(left, 1, "no temporary file is left");
            let unsaved = listed()["unsaved"].clone();
            assert_eq!(unsaved["code"], "save-failed", "{unsaved}");
            let told = unsaved["error"].as_str().unwrap();
            assert!(told.contains(why) && told.ends_with("it keeps its last save"), "{unsaved}");
            release("s3");
        }
        // The next session starts from the last save.
        let used = reserve("s4").unwrap();
        restore(&app, "s4", &used).await.unwrap();
        assert_eq!(browser.lock().unwrap().restored, logins);
    }
}
