//! The installation's egresses: the upstream proxies a session may leave through instead of directly, each with the
//! time zone and languages its browsers then show. The administrator adds, changes and removes them in the dashboard;
//! never in the demo.
//!
//! Adding one, or giving one a new proxy, checks it first (see [`check`]) and saves it only if the check passed: the
//! check also learns the country the proxy's traffic leaves from, which the egress keeps, and, unless the
//! administrator states them, its browsers' time zone and languages follow from that country (`locale::suggested`).
//!
//! `egresses.json` in the state directory holds them (mode 0600, replaced atomically), their credentials included.
//! No answer, log line or error ever holds a credential or the URL it came in: the administrator's listing shows each
//! egress's scheme, host and port and whether it has a user name and a password. At most [`MAX_EGRESSES`].
//!
//! A session starts through an egress only once a check of the very upstream it will use passed: just before, or, for
//! an upstream without the `{session}` placeholder (the same for every session), within the last [`PASS_FRESH`]. A
//! starting or running session holds its egress until it is stopped, so the egress cannot be deleted meanwhile; a new
//! proxy reaches only the sessions started after it.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::Locale;
use crate::tokens::{fresh_id, id_text, parse_id};

/// Egresses at once.
pub const MAX_EGRESSES: usize = 32;
const FILE: &str = "egresses.json";
/// The largest file read.
const MAX_FILE: u64 = 256 * 1024;
/// How long a passed check stands for a session's start, through an upstream the same for every session.
pub const PASS_FRESH: Duration = Duration::from_secs(30);
/// The longest name.
const MAX_NAME: usize = 32;
/// The longest proxy URL taken.
const MAX_PROXY: usize = 2048;
/// How long the admin waits for a check: the controller's own bound (30 s) and the removal of what it made.
const CHECK_DEADLINE: Duration = Duration::from_secs(60);
/// Why a check may fail, as the controller passes them on.
const REASONS: [&str; 7] = ["unreachable", "timeout", "auth", "refused", "tls", "protocol", "unexpected"];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    id: String,
    name: String,
    upstream: egress::Upstream,
    timezone: String,
    languages: Vec<String>,
    /// The country its last passed check found its traffic leaving from; none before one.
    country: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stores {
    egresses: Vec<Stored>,
}

/// One egress a session may leave through.
#[derive(Clone)]
pub struct Egress {
    pub id: String,
    pub name: String,
    pub upstream: egress::Upstream,
    pub locale: Locale,
    pub country: Option<String>,
    /// Changes whenever its proxy does: a passed check stands for this version only.
    version: u64,
}

/// Why an egress operation was refused, or failed.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// No egress has that id (or name).
    NoSuch,
    BadName,
    NameTaken,
    /// The proxy URL, and what is wrong with it (never what was written).
    BadProxy(&'static str),
    BadTimezone,
    BadLanguages,
    TooMany,
    /// These sessions use it.
    InUse(Vec<String>),
    Unsaved,
}

impl Refused {
    /// The status and body a refusal is answered with: `{"error": why}`, and the sessions that use it for one in use.
    pub fn answer(&self) -> (StatusCode, Value) {
        let (status, why) = match self {
            Refused::NoSuch => (StatusCode::NOT_FOUND, "no such egress".to_owned()),
            Refused::BadName => (StatusCode::BAD_REQUEST, format!("name it in 1 to {MAX_NAME} of a-z, 0-9 and -")),
            Refused::NameTaken => (StatusCode::CONFLICT, "another egress has that name".to_owned()),
            Refused::BadProxy(why) => (StatusCode::BAD_REQUEST, format!("the proxy: {why}")),
            Refused::BadTimezone => {
                (StatusCode::BAD_REQUEST, "the time zone is not an IANA name such as America/Sao_Paulo".to_owned())
            }
            Refused::BadLanguages => (
                StatusCode::BAD_REQUEST,
                format!("languages must be 1 to {} tags such as pt-BR", locale::MAX_LANGUAGES),
            ),
            Refused::TooMany => (StatusCode::CONFLICT, format!("at most {MAX_EGRESSES} egresses; delete one first")),
            Refused::InUse(sessions) => {
                let why = format!("sessions use this egress ({}); stop them first", sessions.join(", "));
                return (StatusCode::CONFLICT, json!({"error": why, "sessions": sessions}));
            }
            Refused::Unsaved => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "the egresses could not be saved on the server; its log says why".to_owned(),
            ),
        };
        (status, json!({"error": why}))
    }
}

/// A proxy URL as the administrator gives it: `scheme://[user[:password]@]host:port`, checked as the controller and
/// the session's proxy will check it again, its `{session}` placeholder included (a session's id is 12 characters,
/// which must still leave a valid user name).
pub fn proxy(text: &str) -> Result<egress::Upstream, Refused> {
    if text.len() > MAX_PROXY {
        return Err(Refused::BadProxy("longer than 2048 bytes"));
    }
    let upstream = egress::Upstream::parse_url(text.trim()).map_err(Refused::BadProxy)?;
    upstream.for_session("0123456789ab").map_err(Refused::BadProxy)?;
    Ok(upstream)
}

fn valid_name(name: &str) -> bool {
    (1..=MAX_NAME).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_country(code: &str) -> bool {
    code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// A time zone and languages, each as given or else as suggested for `country`, checked.
pub fn locale_for(timezone: Option<String>, languages: Option<Vec<String>>, country: &str) -> Result<Locale, Refused> {
    let (suggested_zone, suggested_languages) = locale::suggested(country);
    locale_of(timezone.unwrap_or(suggested_zone), languages.unwrap_or(suggested_languages))
}

fn locale_of(timezone: String, languages: Vec<String>) -> Result<Locale, Refused> {
    if !locale::valid_timezone(&timezone) {
        return Err(Refused::BadTimezone);
    }
    if !locale::valid_languages(&languages) {
        return Err(Refused::BadLanguages);
    }
    Ok(Locale { timezone, languages })
}

/// A change to an egress: what is given replaces what it had, a new upstream its address and credentials whole, with
/// the country its check found; the time zone and languages each as given, else as they are.
#[derive(Default)]
pub struct Change {
    pub name: Option<String>,
    pub upstream: Option<(egress::Upstream, String)>,
    pub timezone: Option<String>,
    pub languages: Option<Vec<String>>,
}

/// What one check found: the upstream carried the trace page's request, from `ip` in `country`; or why not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Checked {
    Passed { latency_ms: u64, ip: String, country: String },
    Failed { reason: String, detail: String },
}

impl Checked {
    /// The check's answer: `{"ok":true,"latency_ms","exit":{"ip","country","country_name"},"suggested":{"timezone",
    /// "languages"}}`, or `{"ok":false,"reason","detail"}`.
    pub fn answer(&self) -> Value {
        match self {
            Checked::Passed { latency_ms, ip, country } => {
                let (timezone, languages) = locale::suggested(country);
                json!({
                    "ok": true,
                    "latency_ms": latency_ms,
                    "exit": {"ip": ip, "country": country, "country_name": country_name(country)},
                    "suggested": {"timezone": timezone, "languages": languages},
                })
            }
            Checked::Failed { reason, detail } => json!({"ok": false, "reason": reason, "detail": detail}),
        }
    }
}

fn country_name(code: &str) -> Option<&'static str> {
    locale::country(code).map(|c| c.name)
}

/// Why a check did not run.
#[derive(Debug, PartialEq, Eq)]
pub enum Unchecked {
    /// As many checks as the controller allows are under way.
    Busy,
    Failed(String),
}

impl Unchecked {
    pub fn answer(&self) -> (StatusCode, String) {
        match self {
            Unchecked::Busy => {
                (StatusCode::SERVICE_UNAVAILABLE, "other checks are under way; try again shortly".into())
            }
            Unchecked::Failed(why) => (StatusCode::BAD_GATEWAY, format!("the check could not run: {why}")),
        }
    }
}

/// Checks `upstream` (its placeholder already filled) through the controller (`check_egress`), and holds the
/// controller's answer to its shape again.
pub async fn check(
    controller: &crate::controller::Controller,
    upstream: &egress::Upstream,
) -> Result<Checked, Unchecked> {
    let asked = tokio::time::timeout(CHECK_DEADLINE, controller.check_egress(upstream)).await;
    let outcome = match asked {
        Err(_) => return Err(Unchecked::Failed("no answer in time".into())),
        Ok(Err(why)) if why.contains("egress checks are under way") => return Err(Unchecked::Busy),
        Ok(Err(why)) => return Err(Unchecked::Failed(why)),
        Ok(Ok(outcome)) => outcome,
    };
    let text = |key: &str| outcome.get(key).and_then(Value::as_str).map(str::to_owned);
    let checked = match outcome.get("ok") {
        Some(Value::Bool(true)) => {
            let latency_ms = outcome.get("latency_ms").and_then(Value::as_u64);
            let ip = text("ip").filter(|ip| ip.parse::<std::net::IpAddr>().is_ok());
            let country = text("country").filter(|c| valid_country(c));
            latency_ms.zip(ip).zip(country).map(|((latency_ms, ip), country)| Checked::Passed {
                latency_ms,
                ip,
                country,
            })
        }
        Some(Value::Bool(false)) => {
            let reason = text("reason").filter(|r| REASONS.contains(&r.as_str()));
            let detail = text("detail").filter(|d| d.len() <= 200);
            reason.zip(detail).map(|(reason, detail)| Checked::Failed { reason, detail })
        }
        _ => None,
    };
    checked.ok_or_else(|| Unchecked::Failed("the controller's answer is malformed".into()))
}

pub struct Egresses {
    file: PathBuf,
    egresses: Vec<Egress>,
    /// The egress each starting or running session holds, by the session's id.
    in_use: HashMap<String, String>,
    /// The last check recorded of each egress, by its id: the version checked, when the check began, and whether it
    /// passed.
    checks: HashMap<String, (u64, Instant, bool)>,
    /// The last version given out.
    versions: u64,
}

impl Egresses {
    /// No egresses (the demo's), never written.
    pub fn empty(state_dir: &Path) -> Egresses {
        Egresses {
            file: state_dir.join(FILE),
            egresses: Vec::new(),
            in_use: HashMap::new(),
            checks: HashMap::new(),
            versions: 0,
        }
    }

    /// The stored egresses, none when there is no file yet. The file is read only if it is a regular file (not a link;
    /// opened without blocking, so a FIFO cannot stall the start) owned by the admin's uid, which no one else may use,
    /// at most 256 KiB; one that cannot be read whole, or holds anything the rules refuse, is refused, and its errors
    /// never quote it.
    pub fn load(state_dir: &Path) -> Result<Egresses, String> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let mut loaded = Egresses::empty(state_dir);
        let shown = loaded.file.display().to_string();
        let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
        let opened = std::fs::OpenOptions::new().read(true).custom_flags(flags.bits() as i32).open(&loaded.file);
        let mut file = match opened {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(loaded),
            Err(e) => return Err(format!("{shown}: cannot open it: {}", e.kind())),
        };
        let meta = file.metadata().map_err(|e| format!("{shown}: cannot inspect it: {}", e.kind()))?;
        if !meta.is_file() || meta.uid() != rustix::process::getuid().as_raw() || meta.mode() & 0o077 != 0 {
            return Err(format!("{shown}: not a regular file of the admin's alone (mode 0600)"));
        }
        let mut bytes = Vec::new();
        file.by_ref().take(MAX_FILE + 1).read_to_end(&mut bytes).map_err(|e| format!("{shown}: {}", e.kind()))?;
        if bytes.len() as u64 > MAX_FILE {
            return Err(format!("{shown}: larger than 256 KiB"));
        }
        // Only the position of a JSON error: its message may quote a credential.
        let stored: Stores = serde_json::from_slice(&bytes)
            .map_err(|e| format!("{shown}: not the expected JSON (line {}, column {})", e.line(), e.column()))?;
        if stored.egresses.len() > MAX_EGRESSES {
            return Err(format!("{shown}: more than {MAX_EGRESSES} egresses"));
        }
        for (at, s) in stored.egresses.into_iter().enumerate() {
            let malformed = |what: &str| format!("{shown}: egress {}: {what}", at + 1);
            if parse_id(&s.id).is_none() || loaded.egresses.iter().any(|e| e.id == s.id) {
                return Err(malformed("a malformed or repeated id"));
            }
            if !valid_name(&s.name) || loaded.egresses.iter().any(|e| e.name == s.name) {
                return Err(malformed("a malformed or repeated name"));
            }
            s.upstream.validate().and_then(|()| s.upstream.for_session("0123456789ab").map(drop)).map_err(malformed)?;
            let locale =
                locale_of(s.timezone, s.languages).map_err(|_| malformed("a malformed time zone or languages"))?;
            if !s.country.as_deref().is_none_or(valid_country) {
                return Err(malformed("a malformed country"));
            }
            let version = loaded.version();
            let (id, name, upstream, country) = (s.id, s.name, s.upstream, s.country);
            loaded.egresses.push(Egress { id, name, upstream, locale, country, version });
        }
        Ok(loaded)
    }

    /// Writes every egress, now.
    fn save(&self) -> Result<(), String> {
        let egresses = self
            .egresses
            .iter()
            .map(|e| Stored {
                id: e.id.clone(),
                name: e.name.clone(),
                upstream: e.upstream.clone(),
                timezone: e.locale.timezone.clone(),
                languages: e.locale.languages.clone(),
                country: e.country.clone(),
            })
            .collect();
        let bytes = serde_json::to_vec_pretty(&Stores { egresses }).map_err(|_| "the egresses could not be encoded")?;
        crate::private::replace(&self.file, &bytes)
    }

    /// Writes every egress, or puts `undo` back when that fails. A failure after the file's rename (the directory's
    /// sync) may leave the new contents on disk: the next write, of what is held here, replaces them.
    fn save_or(&mut self, undo: impl FnOnce(&mut Egresses)) -> Result<(), Refused> {
        self.save().map_err(|why| {
            undo(self);
            tracing::error!(%why, "the egresses could not be written");
            Refused::Unsaved
        })
    }

    /// Every egress as the administrator sees it: never a credential.
    pub fn list(&self) -> Vec<Value> {
        self.egresses.iter().map(|e| self.describe(e)).collect()
    }

    fn describe(&self, e: &Egress) -> Value {
        let in_use = self.in_use.values().filter(|id| **id == e.id).count();
        json!({
            "id": e.id,
            "name": e.name,
            "scheme": e.upstream.scheme,
            "host": e.upstream.host,
            "port": e.upstream.port,
            "has_user": e.upstream.username.is_some(),
            "has_password": e.upstream.password.is_some(),
            "timezone": e.locale.timezone,
            "languages": e.locale.languages,
            "country": e.country,
            "country_name": e.country.as_deref().and_then(country_name),
            "in_use": in_use,
        })
    }

    fn position(&self, id: &str) -> Result<usize, Refused> {
        self.egresses.iter().position(|e| e.id == id).ok_or(Refused::NoSuch)
    }

    /// The egress `id`, as it is now.
    pub fn get(&self, id: &str) -> Option<Egress> {
        self.egresses.iter().find(|e| e.id == id).cloned()
    }

    fn name_for(&self, name: &str, except: Option<&str>) -> Result<String, Refused> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(Refused::BadName);
        }
        if self.egresses.iter().any(|e| Some(e.id.as_str()) != except && e.name == name) {
            return Err(Refused::NameTaken);
        }
        Ok(name.to_owned())
    }

    fn version(&mut self) -> u64 {
        self.versions += 1;
        self.versions
    }

    /// Whether a new egress named `name` may be added: what to check before checking its proxy.
    pub fn admits(&self, name: &str) -> Result<String, Refused> {
        let name = self.name_for(name, None)?;
        if self.egresses.len() >= MAX_EGRESSES {
            return Err(Refused::TooMany);
        }
        Ok(name)
    }

    /// A new egress through `upstream`, whose check found it leaving from `country`: its listing.
    pub fn create(
        &mut self,
        name: &str,
        upstream: egress::Upstream,
        locale: Locale,
        country: String,
    ) -> Result<Value, Refused> {
        let name = self.admits(name)?;
        let id = id_text(fresh_id(|id| self.egresses.iter().any(|e| e.id == id_text(id))));
        let version = self.version();
        self.egresses.push(Egress { id, name, upstream, locale, country: Some(country), version });
        self.save_or(|egresses| drop(egresses.egresses.pop()))?;
        Ok(self.describe(self.egresses.last().expect("just added")))
    }

    /// Changes the egress `id`: its new listing. Sessions that already use it keep what they started with.
    pub fn update(&mut self, id: &str, change: Change) -> Result<Value, Refused> {
        let at = self.position(id)?;
        let name = change.name.map(|name| self.name_for(&name, Some(id))).transpose()?;
        let current = &self.egresses[at].locale;
        let locale = locale_of(
            change.timezone.unwrap_or_else(|| current.timezone.clone()),
            change.languages.unwrap_or_else(|| current.languages.clone()),
        )?;
        let version = if change.upstream.is_some() { self.version() } else { self.egresses[at].version };
        let before = self.egresses[at].clone();
        let egress = &mut self.egresses[at];
        if let Some(name) = name {
            egress.name = name;
        }
        if let Some((upstream, country)) = change.upstream {
            egress.upstream = upstream;
            egress.country = Some(country);
        }
        egress.locale = locale;
        egress.version = version;
        self.save_or(|egresses| egresses.egresses[at] = before)?;
        Ok(self.describe(&self.egresses[at]))
    }

    /// Deletes the egress `id`, unless a session holds it.
    pub fn delete(&mut self, id: &str) -> Result<(), Refused> {
        let at = self.position(id)?;
        let mut sessions: Vec<String> = self.in_use.iter().filter(|(_, e)| *e == id).map(|(s, _)| s.clone()).collect();
        if !sessions.is_empty() {
            sessions.sort();
            return Err(Refused::InUse(sessions));
        }
        let egress = self.egresses.remove(at);
        self.save_or(|egresses| egresses.egresses.insert(at, egress))?;
        self.checks.remove(id);
        Ok(())
    }

    /// Takes for `session` the egress with that id (`by_name`: that name), as it is now, until
    /// [`Egresses::release`].
    pub fn reserve(&mut self, key: &str, by_name: bool, session: &str) -> Result<Egress, Refused> {
        let egress = self.egresses.iter().find(|e| if by_name { e.name == key } else { e.id == key });
        let egress = egress.cloned().ok_or(Refused::NoSuch)?;
        self.in_use.insert(session.to_owned(), egress.id.clone());
        Ok(egress)
    }

    /// Gives back the egress `session` holds, if any.
    pub fn release(&mut self, session: &str) {
        self.in_use.remove(session);
    }

    /// Whether a session may start through `egress` without a check of its own: its upstream is the same for every
    /// session, and, as it is, the last check recorded of it passed, and began within [`PASS_FRESH`].
    pub fn passed(&self, egress: &Egress) -> bool {
        !egress.upstream.per_session()
            && self.checks.get(&egress.id).is_some_and(|(version, began, passed)| {
                *passed && *version == egress.version && began.elapsed() < PASS_FRESH
            })
    }

    /// Records what a check of `egress`, as it was, that began at `began` found, unless the egress has changed since or
    /// a check that began later is recorded already: a pass stands for that version, from when it began, and keeps the
    /// country it found; a failure stands until a later pass.
    pub fn record(&mut self, egress: &Egress, began: Instant, checked: &Checked) {
        let Ok(at) = self.position(&egress.id) else { return };
        let later = self.checks.get(&egress.id).is_some_and(|(_, recorded, _)| *recorded > began);
        if self.egresses[at].version != egress.version || later {
            return;
        }
        let passed = matches!(checked, Checked::Passed { .. });
        self.checks.insert(egress.id.clone(), (egress.version, began, passed));
        let Checked::Passed { country, .. } = checked else { return };
        if self.egresses[at].country.as_ref() != Some(country) {
            let before = self.egresses[at].country.replace(country.clone());
            if self.save_or(|egresses| egresses.egresses[at].country = before).is_err() {
                tracing::warn!("the country an egress's check found was not saved");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BERLIN: &str = "socks5://user-{session}:hunter2@proxy.example.net:1080";

    fn berlin() -> Locale {
        Locale { timezone: "Europe/Berlin".into(), languages: vec!["de-DE".into(), "de".into()] }
    }

    /// Adds `name` through `url`, found in Germany.
    fn add(egresses: &mut Egresses, name: &str, url: &str) -> Result<Value, Refused> {
        egresses.create(name, proxy(url)?, berlin(), "DE".into())
    }

    #[test]
    fn egresses_are_kept_private_and_atomically_and_read_back_whole() {
        let dir = crate::private::test_dir("egresses");
        let mut egresses = Egresses::load(&dir).unwrap();
        assert!(egresses.list().is_empty(), "no file yet: none");
        let id = add(&mut egresses, "berlin", BERLIN).unwrap()["id"].as_str().unwrap().to_owned();
        let path = dir.join(FILE);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(&*dir).unwrap().count(), 1, "no temporary file is left");
        let loaded = Egresses::load(&dir).unwrap();
        let back = loaded.get(&id).unwrap();
        assert_eq!((back.name.as_str(), back.upstream.password.as_deref()), ("berlin", Some("hunter2")));
        assert_eq!((back.locale.clone(), back.country.as_deref()), (berlin(), Some("DE")));
        // Only a regular file of the admin's alone is read.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(Egresses::load(&dir).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let moved = dir.join("elsewhere.json");
        std::fs::rename(&path, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &path).unwrap();
        assert!(Egresses::load(&dir).is_err(), "a link is not followed");
        std::fs::remove_file(&path).unwrap();
        rustix::fs::mknodat(rustix::fs::CWD, &path, rustix::fs::FileType::Fifo, 0o600.into(), 0).unwrap();
        assert!(Egresses::load(&dir).is_err(), "a FIFO is refused at once");
        // A write that fails leaves the egresses as they were.
        std::fs::remove_dir_all(&*dir).unwrap();
        assert_eq!(add(&mut egresses, "paris", BERLIN), Err(Refused::Unsaved));
        assert_eq!(egresses.list().len(), 1);
        let rename = Change { name: Some("bonn".into()), ..Change::default() };
        assert_eq!(egresses.update(&id, rename), Err(Refused::Unsaved));
        assert_eq!(egresses.get(&id).unwrap().name, "berlin");
    }

    #[test]
    fn no_credential_is_listed_and_a_malformed_file_is_never_quoted() {
        let dir = crate::private::test_dir("egresses");
        let mut egresses = Egresses::load(&dir).unwrap();
        add(&mut egresses, "berlin", BERLIN).unwrap();
        let listed = serde_json::to_string(&egresses.list()).unwrap();
        assert!(!listed.contains("hunter2") && !listed.contains("user-"), "{listed}");
        for shown in [r#""has_user":true"#, r#""has_password":true"#, r#""scheme":"socks5""#, r#""port":1080"#] {
            assert!(listed.contains(shown), "{listed}");
        }
        assert!(listed.contains(r#""country":"DE""#) && listed.contains(r#""country_name":"Germany""#), "{listed}");
        let file = std::fs::read_to_string(dir.join(FILE)).unwrap();
        for broken in [
            r#"{"egresses": [{"id": "hunter2"}]}"#.to_owned(),
            file.replace("1080", "0"),
            file.replace("Europe/Berlin", "hunter2"),
            file.replace(r#""DE""#, r#""hunter2""#),
            format!("{} ", file.replace("berlin", "hunter2 proxy.example")),
            "hunter2".to_owned(),
        ] {
            crate::private::replace(&dir.join(FILE), broken.as_bytes()).unwrap();
            let why = Egresses::load(&dir).err().unwrap_or_else(|| panic!("accepted {broken}"));
            assert!(!why.contains("hunter2") && !why.contains("proxy.example"), "{why}");
        }
    }

    #[test]
    fn every_rule_is_checked_and_its_error_never_quotes_the_proxy() {
        let dir = crate::private::test_dir("egresses");
        let mut egresses = Egresses::load(&dir).unwrap();
        add(&mut egresses, "berlin", BERLIN).unwrap();
        for (name, refused) in
            [("Berlin!", Refused::BadName), ("", Refused::BadName), (&"a".repeat(33)[..], Refused::BadName)]
        {
            assert_eq!(egresses.admits(name), Err(refused));
        }
        assert_eq!(egresses.admits("berlin"), Err(Refused::NameTaken));
        assert_eq!(locale_for(Some("Europe/Nowhere".into()), None, "DE"), Err(Refused::BadTimezone));
        assert_eq!(locale_for(None, Some(vec![]), "DE"), Err(Refused::BadLanguages));
        assert_eq!(locale_for(None, Some(vec!["en;q=0.9".into()]), "DE"), Err(Refused::BadLanguages));
        // What is not given follows the country; what is, stays.
        let suggested = locale_for(None, None, "BR").unwrap();
        assert_eq!((suggested.timezone.as_str(), suggested.languages[0].as_str()), ("America/Sao_Paulo", "pt-BR"));
        assert_eq!(locale_for(Some("UTC".into()), None, "BR").unwrap().languages[0], "pt-BR");
        for url in [
            "ftp://hunter2@proxy.example.net:21",
            "http://hunter2@proxy.example.net",
            "socks5://hunter2@proxy.example.net:1080",
            "http://u:hunter2@proxy example:8080",
        ] {
            let refused = proxy(url).unwrap_err();
            assert!(matches!(refused, Refused::BadProxy(_)), "{url}: {refused:?}");
            let (status, body) = refused.answer();
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(!body.to_string().contains("hunter2") && !body.to_string().contains("proxy.example"), "{body}");
        }
        let near = format!("http://{}{{session}}:pw@proxy.net:8080", "u".repeat(255 - 9));
        assert!(matches!(proxy(&near), Err(Refused::BadProxy(_))), "the session id must fit");
        for n in 1..MAX_EGRESSES {
            add(&mut egresses, &format!("e{n}"), "http://proxy.net:8080").unwrap();
        }
        assert_eq!(egresses.admits("one-more"), Err(Refused::TooMany));
        assert_eq!(add(&mut egresses, "one-more", "http://proxy.net:8080"), Err(Refused::TooMany));
        assert_eq!(Egresses::load(&dir).unwrap().list().len(), MAX_EGRESSES);
    }

    #[test]
    fn a_new_proxy_replaces_the_credentials_whole_and_an_egress_held_is_never_deleted() {
        let dir = crate::private::test_dir("egresses");
        let mut egresses = Egresses::load(&dir).unwrap();
        let id = add(&mut egresses, "berlin", BERLIN).unwrap()["id"].as_str().unwrap().to_owned();
        let change = Change {
            upstream: Some((proxy("http://proxy.example.net:3128").unwrap(), "FR".into())),
            ..Change::default()
        };
        let changed = egresses.update(&id, change).unwrap();
        assert_eq!((&changed["has_user"], &changed["has_password"]), (&json!(false), &json!(false)));
        assert_eq!((&changed["country"], &changed["timezone"]), (&json!("FR"), &json!("Europe/Berlin")), "as it was");
        // Each field given replaces its own alone, merged into the egress as it is then.
        let zoned = egresses.update(&id, Change { timezone: Some("UTC".into()), ..Change::default() }).unwrap();
        assert_eq!((&zoned["timezone"], &zoned["languages"]), (&json!("UTC"), &json!(["de-DE", "de"])));
        let spoken = egresses.update(&id, Change { languages: Some(vec!["fr".into()]), ..Change::default() }).unwrap();
        assert_eq!((&spoken["timezone"], &spoken["languages"]), (&json!("UTC"), &json!(["fr"])));
        let bad = Change { timezone: Some("Mars/Base".into()), ..Change::default() };
        assert_eq!(egresses.update(&id, bad), Err(Refused::BadTimezone));
        let renamed = egresses.update(&id, Change { name: Some("bonn".into()), ..Change::default() }).unwrap();
        assert_eq!((&renamed["name"], &renamed["scheme"]), (&json!("bonn"), &json!("http")));
        assert_eq!(egresses.update("0123456789abcdef", Change::default()), Err(Refused::NoSuch));
        let held = egresses.reserve("bonn", true, "0123456789ab").unwrap();
        assert_eq!(held.id, id);
        assert_eq!(egresses.reserve(&id, false, "0123456789ac").unwrap().name, "bonn");
        assert!(egresses.reserve("bonn", false, "0123456789ad").is_err(), "an id is not a name");
        assert_eq!(egresses.list()[0]["in_use"], 2);
        let refused = egresses.delete(&id).unwrap_err();
        assert_eq!(refused, Refused::InUse(vec!["0123456789ab".into(), "0123456789ac".into()]));
        assert_eq!(refused.answer().0, StatusCode::CONFLICT);
        assert_eq!(refused.answer().1["sessions"], json!(["0123456789ab", "0123456789ac"]));
        egresses.release("0123456789ab");
        egresses.release("0123456789ac");
        egresses.delete(&id).unwrap();
        assert_eq!(egresses.delete(&id), Err(Refused::NoSuch));
        assert!(Egresses::load(&dir).unwrap().list().is_empty());
        assert_eq!(egresses.reserve(&id, false, "0123456789ab").err(), Some(Refused::NoSuch));
    }

    #[test]
    fn a_pass_stands_for_its_version_alone_briefly_and_never_for_a_per_session_upstream() {
        let dir = crate::private::test_dir("egresses");
        let mut egresses = Egresses::load(&dir).unwrap();
        let id = add(&mut egresses, "shared", "http://u:pw@proxy.net:8080").unwrap()["id"].as_str().unwrap().to_owned();
        let pass = |country: &str| Checked::Passed { latency_ms: 5, ip: "203.0.113.9".into(), country: country.into() };
        let checked = egresses.get(&id).unwrap();
        assert!(!egresses.passed(&checked));
        egresses.record(&checked, Instant::now(), &pass("NL"));
        assert!(egresses.passed(&egresses.get(&id).unwrap()));
        assert_eq!(Egresses::load(&dir).unwrap().get(&id).unwrap().country.as_deref(), Some("NL"), "kept");
        // A new name keeps it; a new proxy does not, nor does a check of the old one that ends after.
        egresses.update(&id, Change { name: Some("bonn".into()), ..Change::default() }).unwrap();
        assert!(egresses.passed(&egresses.get(&id).unwrap()));
        let change =
            Change { upstream: Some((proxy("http://proxy.net:8080").unwrap(), "DE".into())), ..Change::default() };
        egresses.update(&id, change).unwrap();
        assert!(!egresses.passed(&egresses.get(&id).unwrap()));
        egresses.record(&checked, Instant::now(), &pass("NL"));
        assert!(!egresses.passed(&egresses.get(&id).unwrap()));
        assert_eq!(egresses.get(&id).unwrap().country.as_deref(), Some("DE"), "not the old version's");
        let now = egresses.get(&id).unwrap();
        egresses.record(&now, Instant::now(), &pass("DE"));
        assert!(egresses.passed(&now));
        egresses.record(&now, Instant::now(), &Checked::Failed { reason: "timeout".into(), detail: "x".into() });
        assert!(!egresses.passed(&now), "a failure forgets the pass");
        // A pass whose check began long ago is stale however late it came; one older than the last recorded counts
        // for nothing.
        egresses.checks.remove(&id);
        egresses.record(&now, Instant::now() - PASS_FRESH, &pass("DE"));
        assert!(!egresses.passed(&now), "stale");
        let began = Instant::now();
        egresses.record(&now, began, &Checked::Failed { reason: "auth".into(), detail: "x".into() });
        egresses.record(&now, began - Duration::from_secs(1), &pass("DE"));
        assert!(!egresses.passed(&now), "an earlier check's pass does not undo a later failure");
        // Each session leaves through an upstream of its own: a pass for one is none for the next.
        let per_session = add(&mut egresses, "berlin", BERLIN).unwrap()["id"].as_str().unwrap().to_owned();
        let per_session = egresses.get(&per_session).unwrap();
        egresses.record(&per_session, Instant::now(), &pass("DE"));
        assert!(!egresses.passed(&per_session));
    }

    #[test]
    fn a_check_answers_with_its_exit_and_suggestion_or_its_reason() {
        let passed = Checked::Passed { latency_ms: 412, ip: "203.0.113.9".into(), country: "BR".into() }.answer();
        assert_eq!(
            passed,
            json!({"ok": true, "latency_ms": 412, "exit": {"ip": "203.0.113.9", "country": "BR", "country_name": "Brazil"},
                   "suggested": {"timezone": "America/Sao_Paulo", "languages": ["pt-BR", "pt", "en-US", "en"]}})
        );
        let unknown = Checked::Passed { latency_ms: 1, ip: "203.0.113.9".into(), country: "T1".into() }.answer();
        assert_eq!(
            (&unknown["exit"]["country_name"], &unknown["suggested"]["timezone"]),
            (&Value::Null, &json!("UTC"))
        );
        let failed =
            Checked::Failed { reason: "auth".into(), detail: "the proxy refused the user name or password".into() };
        assert_eq!(
            failed.answer(),
            json!({"ok": false, "reason": "auth", "detail": "the proxy refused the user name or password"})
        );
    }
}
