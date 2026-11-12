//! Sessions that come back after the admin was stopped (an update, a restart). Never in the demo.
//!
//! As the admin is stopped (`service::save_before_exit`), every session that runs is kept before it ends: its browser's
//! profile, saved as a saved session's is (into that saved session, if it runs from one, else into an archive of its
//! own here), and a record of what starts it again: its page, its settings, who started it, where it leaves from, and
//! what a person typed in it (its redactions), so the agent reads no more of that after than before. Both are sealed
//! with a key of that stop's own, itself sealed for the recordings' public key (see `replay`): the admin keeps a
//! session with no one logged in, and only the administrator's password opens it again. Without that key pair (no
//! password ever made it) nothing is kept, and the sessions end as they did.
//!
//! `resume/<id>.key`, `resume/<id>.json` and, for a session that held no saved one, `resume/<id>.bin` in the state
//! directory (mode 0600), `<id>` the session's id as it stopped; the record is written last, so one that is there has
//! all it names. What waits longer than [`KEPT_FOR`] goes as the admin starts.
//!
//! Once the password opened the recordings ([`start_all`]), each session starts again, under a new id (the stopped
//! run's timeline and replay keep the old one): from its saved session, or from its own archive, its tabs as Chrome
//! restores them, its pages loaded again. Whoever started it must still be allowed to: a revoked token, a saved session
//! deleted or no longer theirs, an egress gone, refuse the start as they would any, and what was kept goes. A start
//! refused for want of a place ([`Code::TooManySessions`]), or because the admin was stopping again, keeps it and is
//! tried again; a record that opens but says what this release does not read stays for one that does. A session someone
//! stops meanwhile starts no more. A session that had a chat open opens it again, with the same account and in the same
//! place of its home, where its program finds the conversation it had. Until then the ids it had are told apart from ones that never were: an agent that
//! names one hears that the session waits, or which session it is now.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::{App, Features, Owner, Session};
use crate::coded::{Code, Coded};
use crate::keys::{self, DataKey};
use crate::profiles::ProfileUse;
use crate::service::{EgressChoice, Start};

const DIR: &str = "resume";
/// How long what was kept of a session waits for the password.
const KEPT_FOR: Duration = Duration::from_secs(30 * 24 * 3600);
/// The ids a session is remembered by: its latest stops'.
const MAX_WAS: usize = 8;
/// The longest record read: its redactions are bounded (see `redact`), the rest is small.
const MAX_RECORD: u64 = 8 * 1024 * 1024;
/// Sessions started again at once.
const AT_ONCE: usize = 4;
/// Starts that failed, as the administrator's page lists them.
const MAX_FAILED: usize = 32;

/// Who started a session, as its record keeps them: the principals that outlast the admin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Who {
    #[default]
    Admin,
    Token(u64),
    Local,
}

impl Who {
    fn of(owner: Owner) -> Option<Who> {
        match owner {
            Owner::Admin => Some(Who::Admin),
            Owner::Token(id) => Some(Who::Token(id)),
            Owner::Local => Some(Who::Local),
            Owner::Visitor(_) | Owner::Chat(..) => None,
        }
    }

    fn owner(self) -> Owner {
        match self {
            Who::Admin => Owner::Admin,
            Who::Token(id) => Owner::Token(id),
            Who::Local => Owner::Local,
        }
    }
}

/// A session's features as its record keeps them, each by its name: its own shape, apart from the API's, so a record
/// stays readable whatever the API comes to refuse. One a record lacks is off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct KeptFeatures {
    keyboard_pointer: bool,
    clipboard: bool,
    uploads: bool,
    downloads: bool,
    sound: bool,
    recording: bool,
    replay: bool,
}

impl From<Features> for KeptFeatures {
    fn from(f: Features) -> KeptFeatures {
        let Features { keyboard_pointer, clipboard, uploads, downloads, sound, recording, replay } = f;
        KeptFeatures { keyboard_pointer, clipboard, uploads, downloads, sound, recording, replay }
    }
}

impl From<KeptFeatures> for Features {
    fn from(f: KeptFeatures) -> Features {
        let KeptFeatures { keyboard_pointer, clipboard, uploads, downloads, sound, recording, replay } = f;
        Features { keyboard_pointer, clipboard, uploads, downloads, sound, recording, replay }
    }
}

/// What starts a session again. Every field has a default and an unknown one is passed over, so a record opens in the
/// release after the one that wrote it; one that still cannot be read (a principal or a shape this release does not
/// know) is held, not removed (see [`Then::Held`]).
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Record {
    url: String,
    devtools: bool,
    cdp: bool,
    /// Its egress's id; none is direct.
    egress: Option<String>,
    kiosk: bool,
    lane: Option<u8>,
    owner: Who,
    /// Started for its agent (see `Session::started_by_agent`).
    agent: bool,
    features: KeptFeatures,
    /// The saved session it ran from, by its id: its browser's profile was saved there.
    profile: Option<String>,
    size: (u32, u32),
    languages: Option<Vec<String>>,
    redactions: crate::redact::Kept,
    /// What its timeline called it.
    label: String,
    /// When it stopped, in milliseconds.
    stopped: u64,
    /// The ids it had before this stop's, oldest first.
    was: Vec<String>,
    /// What its egress keeps one exit address by (see `service::start`), and where its chat works (`Session::place`).
    route: String,
    /// The account of the chat it had open, by its id: opened again with it.
    chat: Option<String>,
}

impl Record {
    fn of(session: &Session) -> Option<Record> {
        let state = session.state.lock().unwrap();
        if state.failed.is_some() {
            return None;
        }
        let origin = &session.origin;
        Some(Record {
            url: session.url.clone(),
            devtools: session.devtools,
            cdp: origin.cdp,
            egress: session.egress_id.clone(),
            kiosk: origin.kiosk,
            lane: origin.lane,
            owner: Who::of(session.owner)?,
            agent: origin.for_agent,
            features: session.features.into(),
            profile: session.profile.as_ref().map(|p| p.id.clone()),
            size: origin.size,
            languages: origin.languages.clone(),
            redactions: state.redactions.kept(),
            label: crate::service::label(session),
            stopped: crate::timeline::now_ms(),
            was: origin.was.clone(),
            route: origin.route.clone(),
            chat: session.chat.lock().unwrap().as_ref().map(|chat| chat.account.id.clone()),
        })
    }
}

/// How an id that is no running session's stands.
#[derive(Debug, PartialEq, Eq)]
pub enum Was {
    /// A session that ran under it waits to start again: for the password, or once that came, for a place.
    Waiting,
    /// It runs again, under this id.
    Resumed(String),
}

#[derive(Default)]
struct Inner {
    /// The ids kept, each with every file its record needs.
    waiting: BTreeSet<String>,
    /// The session each earlier id is now.
    resumed: HashMap<String, String>,
    /// Starts that failed, for the administrator's page: the session's label, and why.
    failed: Vec<Value>,
    /// Ids whose record could not be removed once it had served: tried again, and never started twice by this admin.
    unremoved: BTreeSet<String>,
}

/// What was kept of the sessions that ran as the admin stopped, and what became of them.
pub struct Resumes {
    dir: PathBuf,
    inner: Mutex<Inner>,
    /// One pass at a time starts them again.
    pass: tokio::sync::Mutex<()>,
}

impl Resumes {
    /// What `state_dir`'s `resume/` keeps: a record with its key, and nothing else (a file a stop left half written, a
    /// record that waited longer than [`KEPT_FOR`], and whatever belongs to neither go).
    pub fn open(state_dir: &Path) -> Resumes {
        let dir = state_dir.join(DIR);
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_owned());
            }
        }
        let fresh = |id: &str| {
            let age = std::fs::metadata(dir.join(format!("{id}.json"))).and_then(|m| m.modified()).ok();
            age.and_then(|at| at.elapsed().ok()).is_some_and(|age| age < KEPT_FOR)
        };
        let whole = |id: &str| id.parse::<display_protocol::session::SessionId>().is_ok() && fresh(id);
        let waiting: BTreeSet<String> = names
            .iter()
            .filter_map(|name| name.strip_suffix(".json"))
            .filter(|id| names.iter().any(|n| n.strip_suffix(".key") == Some(id)) && whole(id))
            .map(str::to_owned)
            .collect();
        for name in &names {
            let kept = name
                .rsplit_once('.')
                .is_some_and(|(id, kind)| matches!(kind, "json" | "key" | "bin") && waiting.contains(id));
            if !kept {
                let _ = std::fs::remove_file(dir.join(name));
            }
        }
        if !waiting.is_empty() {
            tracing::info!(sessions = waiting.len(), "sessions wait for the password to start again");
        }
        Resumes { dir, inner: Mutex::new(Inner { waiting, ..Inner::default() }), pass: tokio::sync::Mutex::new(()) }
    }

    /// None kept, and none read (the demo's).
    pub fn none(state_dir: &Path) -> Resumes {
        Resumes { dir: state_dir.join(DIR), inner: Mutex::default(), pass: tokio::sync::Mutex::new(()) }
    }

    fn file(&self, id: &str, kind: &str) -> PathBuf {
        self.dir.join(format!("{id}.{kind}"))
    }

    /// How `id`, which no running session has, stands: none for an id that never was a kept session's.
    pub fn was(&self, id: &str) -> Option<Was> {
        let inner = self.inner.lock().unwrap();
        if let Some(now) = inner.resumed.get(id) {
            return Some(Was::Resumed(now.clone()));
        }
        inner.waiting.contains(id).then_some(Was::Waiting)
    }

    /// For the administrator's page: how many sessions wait to start again, and the starts that failed.
    pub fn shown(&self) -> Value {
        let inner = self.inner.lock().unwrap();
        json!({"waiting": inner.waiting.len(), "failed": inner.failed})
    }

    /// Removes what was kept under `id`: its record first and for good (its directory synced), since a record that is
    /// there starts a session, then the rest. False when the record is still there, which the log says why.
    fn forget(&self, id: &str) -> bool {
        let record = self.file(id, "json");
        // Synced whether or not it was there: an earlier try may have removed it and failed only at the sync. A
        // directory that never was holds nothing to sync.
        let absent = |e: &std::io::Error| e.kind() == std::io::ErrorKind::NotFound;
        let removed = match std::fs::remove_file(&record) {
            Err(e) if !absent(&e) => Err(e),
            _ => crate::private::sync_parent(&record).or_else(|e| if absent(&e) { Ok(()) } else { Err(e) }),
        };
        if let Err(e) = removed {
            tracing::error!(was = %id, %e, "what was kept of a session could not be removed; it is tried again");
            return false;
        }
        for kind in ["key", "bin"] {
            let _ = std::fs::remove_file(self.file(id, kind));
        }
        true
    }

    /// The same for a session someone stopped: whatever a stop of the admin kept of it meanwhile, it starts no more.
    pub fn cancel(&self, id: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.waiting.remove(id);
        if !self.forget(id) {
            inner.unremoved.insert(id.to_owned());
        }
    }
}

/// Keeps `session`, which is ending as the admin stops, to start again: called by its stop once nothing acts on its
/// page any more (`quiet`), with how its saved session's own save went (`saved`), before its browser is removed. Whatever
/// fails, nothing is kept of it, and its stop goes on.
pub async fn keep(app: &App, session: &Session, quiet: bool, saved: Option<bool>) {
    if app.config.demo.is_some() || !quiet {
        return;
    }
    let Some(public) = app.replays.public() else { return };
    // A saved session's own save holds its browser's profile: only one known to be on the disk is worth a record.
    if session.profile.is_some() && saved != Some(true) {
        return;
    }
    let Some(record) = Record::of(session) else { return };
    let id = session.id.clone();
    let resumes = &app.resumes;
    let made = {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&resumes.dir)
    };
    if let Err(e) = made {
        tracing::warn!(session = %id, %e, "the session could not be kept to start again");
        return;
    }
    let key = DataKey::random();
    if session.profile.is_none() {
        let kept = ProfileUse::kept(id.clone(), key.clone(), true);
        match crate::profiles::keep(app, &id, &kept, &resumes.file(&id, "bin")).await {
            Ok(size) => tracing::info!(session = %id, size, "the session's browser was kept to start again"),
            // Escaped (`Debug`): the browser's part of it is a page's to choose.
            Err(why) => {
                tracing::warn!(session = %id, ?why, "the session's browser was not kept; it will not start again");
                let _ = resumes.forget(&id);
                return;
            }
        }
    }
    let written = {
        let (resumes_key, resumes_record) = (resumes.file(&id, "key"), resumes.file(&id, "json"));
        let name = format!("{id}.json");
        tokio::task::spawn_blocking(move || {
            let plain = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
            // The key first: a record is there only with all it needs.
            crate::private::replace(&resumes_key, &keys::seal_to(&public, &key)).map_err(String::from)?;
            crate::private::replace(&resumes_record, &keys::seal(&key, &name, &plain)).map_err(String::from)
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
    };
    if let Err(why) = written {
        tracing::warn!(session = %id, %why, "the session's record was not kept; it will not start again");
        let _ = resumes.forget(&id);
    }
}

/// What becomes of what was kept of a session whose start did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Then {
    /// It goes: it does not open, or its start is refused for good.
    Gone,
    /// It stays, and a later pass tries again: the refusal passes (no place for now, the admin stopping).
    Again,
    /// It stays, untried by this admin: it opens, but says what this release does not read (a later one wrote it).
    Held,
}

/// Why a start that was kept did not happen: what the administrator's page says of it.
struct Failed {
    label: String,
    why: Coded,
    then: Then,
}

/// What was kept under `id`, opened: its record, and the key it and its browser's archive were sealed with.
async fn opened(app: &App, id: &str) -> Result<(Record, DataKey), Failed> {
    let resumes = &app.resumes;
    let unreadable = |what: &str, then| Failed {
        label: String::new(),
        why: Coded::new(Code::ServerFailed, format!("what was kept of the session {what}")),
        then,
    };
    let (key_file, record_file) = (resumes.file(id, "key"), resumes.file(id, "json"));
    let read = tokio::task::spawn_blocking(move || {
        let bounded = |path: &Path, max: u64| {
            std::fs::metadata(path).ok().filter(|m| m.len() <= max).and_then(|_| std::fs::read(path).ok())
        };
        Some((bounded(&key_file, 4096)?, bounded(&record_file, MAX_RECORD)?))
    });
    let Ok(Some((sealed_key, sealed))) = read.await else { return Err(unreadable("could not be read", Then::Gone)) };
    // Sealed for a key pair the password no longer opens (it was set anew): nothing ever opens it.
    let Some(key) = app.replays.open_key(&sealed_key) else {
        return Err(unreadable("opens with another password", Then::Gone));
    };
    let Some(plain) = keys::open(&key, &format!("{id}.json"), &sealed) else {
        return Err(unreadable("does not open: it was changed or cut", Then::Gone));
    };
    // It is whole, and says what this release does not read: kept for one that does.
    match serde_json::from_slice::<Record>(&plain) {
        Ok(record) => Ok((record, key)),
        Err(_) => {
            Err(unreadable("was written by a release this one does not read; it stays for one that does", Then::Held))
        }
    }
}

/// A session that started again, and why the chat it had did not open with it, if it did not.
type Started = (Arc<Session>, Option<Coded>);

/// Opens what was kept under `id` and starts its session again, then the chat it had: its program opens the
/// conversation it last had in the session's place, which the session kept (see `chat`).
async fn start(app: &Arc<App>, id: &str) -> Result<Started, Failed> {
    let resumes = &app.resumes;
    let (record, key) = opened(app, id).await?;
    let label = record.label.clone();
    let mut was = record.was;
    was.push(id.to_owned());
    let earlier = was.len().saturating_sub(MAX_WAS);
    was.drain(..earlier);
    // Its own browser's profile, unless it runs from a saved session, which holds it.
    let kept = record.profile.is_none().then(|| (ProfileUse::kept(id.to_owned(), key, false), resumes.file(id, "bin")));
    let route = if record.route.is_empty() { id.to_owned() } else { record.route };
    let chat = record.chat;
    let start = Start {
        id: crate::service::session_id(),
        url: record.url,
        devtools: record.devtools,
        cdp: record.cdp,
        egress: record.egress.map(EgressChoice::Id),
        kiosk: record.kiosk,
        lane: record.lane,
        owner: record.owner.owner(),
        agent: record.agent,
        features: record.features.into(),
        profile: record.profile,
        size: Some(record.size).filter(|&(width, height)| width > 0 && height > 0),
        languages: record.languages,
        resumed: Some(Resumed { kept, redactions: record.redactions, was, route }),
    };
    let session = crate::service::start(app, start).await.map_err(|refused| Failed {
        label,
        // No place for now, or the admin stopped under it: it waits.
        then: match refused.why.code {
            Code::TooManySessions | Code::AdminStopping => Then::Again,
            _ => Then::Gone,
        },
        why: refused.why,
    })?;
    // Its chat, once it runs: an account removed since, or a chat that does not start, leaves the session as it is.
    let unopened = match chat {
        Some(account) => crate::chat::open(app, &session.id, &account).await.err().map(|(_, why)| why),
        None => None,
    };
    Ok((session, unopened))
}

/// What a session that starts again brings with it (see `service::start`).
pub struct Resumed {
    /// Its browser's profile as it was kept, and the archive that holds it: none when it runs from a saved session.
    pub kept: Option<(ProfileUse, PathBuf)>,
    pub redactions: crate::redact::Kept,
    /// The ids it had, oldest first.
    pub was: Vec<String>,
    pub route: String,
}

/// Starts again every session that waits, once the password opened the recordings: [`AT_ONCE`] at a time, each once.
/// What was kept of one goes as soon as it started, or was refused for good; one refused for want of a place stays, and
/// the next pass (each minute, see `service::keep_archives`) tries it again.
pub async fn start_all(app: &Arc<App>) {
    if app.config.demo.is_some() || !app.replays.unlocked() {
        return;
    }
    // A pass under way does all there is.
    let Ok(_pass) = app.resumes.pass.try_lock() else { return };
    let stopping = || app.stopping.load(std::sync::atomic::Ordering::SeqCst);
    let waiting: Vec<String> = {
        let mut inner = app.resumes.inner.lock().unwrap();
        // A record that had served and could not be removed is tried again: it must not start a session twice.
        let unremoved = std::mem::take(&mut inner.unremoved);
        inner.unremoved = unremoved.into_iter().filter(|id| !app.resumes.forget(id)).collect();
        inner.waiting.iter().cloned().collect()
    };
    if waiting.is_empty() || stopping() {
        return;
    }
    tracing::info!(sessions = waiting.len(), "starting again the sessions that ran as the admin stopped");
    for ids in waiting.chunks(AT_ONCE) {
        // The admin is stopping: what still waits stays for the one that starts next.
        if stopping() {
            break;
        }
        let mut starts = tokio::task::JoinSet::new();
        for id in ids {
            let (app, id) = (app.clone(), id.clone());
            starts.spawn(async move {
                let outcome = start(&app, &id).await;
                (id, outcome)
            });
        }
        while let Some(joined) = starts.join_next().await {
            let Ok((id, outcome)) = joined else { continue };
            settle(app, &id, outcome);
        }
    }
    crate::app::list_changed();
}

/// Records what became of the start kept under `id`.
fn settle(app: &App, id: &str, outcome: Result<Started, Failed>) {
    let resumes = &app.resumes;
    let mut inner = resumes.inner.lock().unwrap();
    // One entry for each session, its latest: a retry that fails again says so once.
    inner.failed.retain(|failed| failed["was"] != id);
    match outcome {
        Ok((session, unopened)) => {
            tracing::info!(session = %session.id, was = %id, "session started again");
            for was in &session.origin.was {
                inner.resumed.insert(was.clone(), session.id.clone());
            }
            inner.waiting.remove(id);
            if !resumes.forget(id) {
                inner.unremoved.insert(id.to_owned());
            }
            // It runs; its chat alone did not open, which the administrator's page says apart.
            if let Some(why) = unopened {
                tracing::warn!(session = %session.id, ?why, "the chat of a session that started again did not open");
                let earliest = inner.failed.len().saturating_sub(MAX_FAILED - 1);
                inner.failed.drain(..earliest);
                inner.failed.push(json!({
                    "was": id,
                    "label": crate::service::label(&session),
                    "code": why.code.as_str(),
                    "error": why.why,
                    "again": false,
                    "chat": true,
                }));
            }
        }
        Err(failed) => {
            // Escaped (`Debug`): its cause may carry a page's or a browser's own words.
            tracing::warn!(was = %id, why = ?failed.why, then = ?failed.then, "a session did not start again");
            if failed.then != Then::Again {
                inner.waiting.remove(id);
            }
            if failed.then == Then::Gone && !resumes.forget(id) {
                inner.unremoved.insert(id.to_owned());
            }
            let earliest = inner.failed.len().saturating_sub(MAX_FAILED - 1);
            inner.failed.drain(..earliest);
            inner.failed.push(json!({
                "was": id,
                "label": failed.label,
                "code": failed.why.code.as_str(),
                "error": failed.why.why,
                "again": failed.then == Then::Again,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Record {
        Record {
            url: "https://example.com/".into(),
            devtools: true,
            egress: Some("0123456789abcdef".into()),
            owner: Who::Token(7),
            agent: true,
            profile: None,
            size: (1600, 900),
            label: "https://example.com/".into(),
            stopped: 1_700_000_000_000,
            was: vec!["aaaaaaaaaaaa".into()],
            route: "aaaaaaaaaaaa".into(),
            ..Record::default()
        }
    }

    #[test]
    fn a_record_opens_only_whole_under_its_own_name_and_key() {
        let key = DataKey::random();
        let plain = serde_json::to_vec(&record()).unwrap();
        let sealed = keys::seal(&key, "0123456789ab.json", &plain);
        let opened = keys::open(&key, "0123456789ab.json", &sealed).unwrap();
        assert_eq!(serde_json::from_slice::<Record>(&opened).unwrap(), record());
        // Another session's name, another key, a changed byte, a cut one: none opens.
        assert!(keys::open(&key, "ba9876543210.json", &sealed).is_none());
        assert!(keys::open(&DataKey::random(), "0123456789ab.json", &sealed).is_none());
        let mut changed = sealed.clone();
        *changed.last_mut().unwrap() ^= 1;
        assert!(keys::open(&key, "0123456789ab.json", &changed).is_none());
        assert!(keys::open(&key, "0123456789ab.json", &sealed[..sealed.len() - 1]).is_none());
        assert!(keys::open(&key, "0123456789ab.json", &[]).is_none());
        // More than one chunk opens too (a record with many redactions).
        let long = vec![7u8; 3 * keys::CHUNK + 5];
        assert_eq!(&keys::open(&key, "long", &keys::seal(&key, "long", &long)).unwrap()[..], &long[..]);
    }

    #[test]
    fn a_record_of_an_earlier_release_opens_and_the_demo_s_principals_are_never_kept() {
        // Fields a later release adds take their defaults; ones it no longer has are ignored.
        let earlier: Record = serde_json::from_str(
            r#"{"url": "https://example.com/", "gone": 1, "features": {"sound": true, "new": 1}}"#,
        )
        .unwrap();
        assert_eq!((earlier.owner, earlier.size, earlier.was.len()), (Who::Admin, (0, 0), 0));
        // A feature it lacks is off; the ones it has are as they were, through the API's own shape and back.
        assert!(earlier.features.sound && !earlier.features.clipboard);
        let all = Features::default();
        assert_eq!(Features::from(KeptFeatures::from(all)), all);
        let kept: crate::redact::Kept = serde_json::from_str("{}").unwrap();
        assert_eq!(kept, crate::redact::Kept::default());
        for owner in [Owner::Admin, Owner::Token(3), Owner::Local] {
            assert_eq!(Who::of(owner).map(Who::owner), Some(owner));
        }
        assert_eq!(Who::of(Owner::Chat(*b"0123456789ab", 1)), None);
    }

    #[tokio::test]
    async fn a_session_is_kept_only_for_the_password_whole_or_not_at_all() {
        use crate::profiles::tests::{fake_controller, frames, kek};
        let socket = crate::private::test_dir("resume-controller");
        let (controller, browser) = fake_controller(&socket);
        let app = crate::app::test_app_with(false, controller);
        let id = "0123456789ab";
        let session = Session::test(id, Owner::Token(7), Features::default());
        {
            let mut state = session.state.lock().unwrap();
            state.redactions.typed("hunter2-wifi");
            state.redactions.boundary();
            state.redactions.screenshots_blocked = true;
        }
        let kept = || {
            let mut names: Vec<String> = std::fs::read_dir(&app.resumes.dir)
                .into_iter()
                .flatten()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        };
        let logins = b"the person's logins ".repeat(5000);
        // No password ever made the key pair: nothing can be kept for it, and the browser is not asked.
        crate::resume::keep(&app, &session, true, None).await;
        assert!(kept().is_empty() && browser.lock().unwrap().ops.is_empty());
        app.replays.unlock(&kek());
        // Its agent could not be stopped, or its browser's profile came cut: nothing of it is left.
        crate::resume::keep(&app, &session, false, None).await;
        let cut = frames(&logins);
        browser.lock().unwrap().saves.push_back(cut[..cut.len() - 4].to_vec());
        crate::resume::keep(&app, &session, true, None).await;
        assert!(kept().is_empty(), "{:?}", kept());
        // Kept whole: its browser's profile, its record, and their key.
        browser.lock().unwrap().saves.push_back(frames(&logins));
        crate::resume::keep(&app, &session, true, None).await;
        assert_eq!(kept(), ["0123456789ab.bin", "0123456789ab.json", "0123456789ab.key"]);
        for name in kept() {
            let held = std::fs::read(app.resumes.dir.join(name)).unwrap();
            assert!(!held.windows(7).any(|w| w == b"hunter2" || w == b"logins "), "sealed, never as it was");
        }
        // The password's key opens it: who started it, what a person typed in it, and what its browser held.
        let (record, key) = opened(&app, id).await.ok().expect("it opens");
        assert_eq!((record.owner, record.url.as_str()), (Who::Token(7), "https://example.com/"));
        let redactions = crate::redact::Redactions::of(record.redactions);
        assert!(redactions.screenshots_blocked && redactions.len() > 0);
        let said = json!({"content": [{"type": "text", "text": "the wifi is hunter2-wifi"}]});
        assert!(!redactions.applied(said).to_string().contains("hunter2"));
        let archive = app.resumes.file(id, "bin");
        let held = ProfileUse::kept(id.to_owned(), key, false);
        crate::profiles::restore_from(&app, "ba9876543210", &held, &archive).await.unwrap();
        assert_eq!(browser.lock().unwrap().restored, logins);
        // A record whole but of a shape this release does not read (a later one's principal) is held for one that does;
        // one changed on the disk opens no more, and goes.
        let record_file = app.resumes.file(id, "json");
        let sealed = keys::seal(&held_key(&app, id), "0123456789ab.json", br#"{"owner": {"team": 3}}"#);
        std::fs::write(&record_file, &sealed).unwrap();
        assert_eq!(opened(&app, id).await.err().map(|failed| failed.then), Some(Then::Held));
        let mut changed = sealed;
        *changed.last_mut().unwrap() ^= 1;
        std::fs::write(&record_file, changed).unwrap();
        assert_eq!(opened(&app, id).await.err().map(|failed| failed.then), Some(Then::Gone));
        // A stop someone asked for takes it all away, and nothing waits.
        app.resumes.cancel(id);
        assert!(kept().is_empty());
    }

    /// The key what was kept under `id` is sealed with, as the password's key opens it.
    fn held_key(app: &App, id: &str) -> DataKey {
        app.replays.open_key(&std::fs::read(app.resumes.file(id, "key")).unwrap()).expect("its key opens")
    }

    #[test]
    fn only_a_whole_fresh_record_waits_and_what_belongs_to_none_goes() {
        let dir = crate::private::test_dir("resume-open");
        let kept = dir.join(DIR);
        std::fs::create_dir(&kept).unwrap();
        let write = |name: &str| std::fs::write(kept.join(name), b"x").unwrap();
        // Whole: a record, its key, its archive. Without its key, cut short, misnamed, or left by a save: gone.
        for name in ["0123456789ab.json", "0123456789ab.key", "0123456789ab.bin"] {
            write(name);
        }
        for name in
            ["aaaaaaaaaaaa.json", "bbbbbbbbbbbb.key", "bbbbbbbbbbbb.bin", "cccccccccccc.bin.0011.tmp", "notes.txt"]
        {
            write(name);
        }
        write("not-an-id.json");
        write("not-an-id.key");
        let resumes = Resumes::open(&dir);
        assert_eq!(resumes.shown(), json!({"waiting": 1, "failed": []}));
        assert_eq!(resumes.was("0123456789ab"), Some(Was::Waiting));
        assert_eq!(resumes.was("aaaaaaaaaaaa"), None);
        let mut left: Vec<String> =
            std::fs::read_dir(&kept).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["0123456789ab.bin", "0123456789ab.json", "0123456789ab.key"]);
        // One that waited too long goes with all it had.
        let old = std::time::SystemTime::now() - KEPT_FOR - Duration::from_secs(60);
        std::fs::File::options().write(true).open(kept.join("0123456789ab.json")).unwrap().set_modified(old).unwrap();
        assert_eq!(Resumes::open(&dir).shown()["waiting"], 0);
        assert_eq!(std::fs::read_dir(&kept).unwrap().count(), 0);
    }
}
