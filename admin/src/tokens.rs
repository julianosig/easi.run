//! Agent tokens: bearer credentials the administrator creates for agents. A token reaches only the agent API and the
//! MCP endpoint, and through them only the sessions started with it (see `app::Owner`); everything else stays behind
//! the administrator's login. The demo has none.
//!
//! A token is `easi_` and 32 random bytes in unpadded base64url, shown once when it is created. Only its SHA-256 is
//! kept, with its id, name and times, in `tokens.json` in the state directory (mode 0600, replaced atomically). A
//! presented token is hashed and compared with every stored hash in constant time. Revoking one marks it revoked (refused
//! at once) and writes that before its sessions are stopped; a write that fails is retried (see [`Tokens::due`]), but a
//! restart before it succeeds accepts the token again. A session being started with it is listed only if the token is
//! still live then (`service::start`), and an agent call checks it again once admitted (`service::call`).
//!
//! A token carries 256 random bits, which no one guesses however fast they try: a failed bearer check is answered
//! like any other, with no rate limit.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::http::{HeaderMap, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

use crate::auth::{hex, sha256, unhex, unix_now};
use crate::private::Unreplaced;

const PREFIX: &str = "easi_";
/// Tokens at once.
pub const MAX_TOKENS: usize = 32;
/// The longest name a token or a profile may have, in characters.
pub const MAX_NAME: usize = 64;
const FILE: &str = "tokens.json";
/// How stale a token's stored last use may get before [`Tokens::due`] writes it again.
const USE_WRITE_EVERY: u64 = 60;

/// One token as stored: never the token itself.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    id: String,
    name: String,
    sha256: String,
    created: u64,
    last_used: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    tokens: Vec<Stored>,
}

struct Token {
    id: u64,
    name: String,
    hash: [u8; 32],
    created: u64,
    last_used: Option<u64>,
    /// The last use as the latest snapshot of the file holds it.
    written_use: Option<u64>,
    /// Revoked, but the file may still list it (writing failed): refused all the same, and written out again (see
    /// [`Tokens::due`]) until a snapshot without it is written.
    revoked: bool,
}

pub struct Tokens {
    file: PathBuf,
    tokens: Vec<Token>,
    /// The version of the latest snapshot of the file, taken under the tokens lock.
    version: u64,
    disk: Arc<Mutex<Disk>>,
}

/// The versions the file holds, locked while one is written.
#[derive(Default)]
struct Disk {
    /// The version in place.
    in_place: u64,
    /// The latest version a power loss keeps: its directory synced too.
    durable: u64,
}

/// The file's contents at one moment. Creating and revoking write theirs under the tokens lock; last uses are written
/// without it, off the request path, so a slow disk never stalls a bearer check. A snapshot older than the file's
/// contents is never written over them.
pub struct Snapshot {
    file: PathBuf,
    bytes: Vec<u8>,
    version: u64,
    disk: Arc<Mutex<Disk>>,
    written: Written,
}

/// What a snapshot holds, once on the file (see [`Tokens::written`]).
pub struct Written {
    /// The revoked tokens it leaves out.
    revoked: Vec<u64>,
    /// Each token's last use.
    uses: Vec<(u64, Option<u64>)>,
}

impl Snapshot {
    /// Writes it, unless the file holds a later one already (which leaves out every token this one does: a revoked
    /// token stays revoked, and holds every use it holds), and returns what it holds, now on the file for good. A
    /// failure once it is in place (syncing its directory) leaves it the file's version all the same, never written
    /// over by an older one; an older one then syncs the directory before it returns.
    pub fn write(self) -> Result<Written, Unreplaced> {
        let mut disk = self.disk.lock().unwrap();
        if disk.in_place < self.version {
            let replaced = crate::private::replace(&self.file, &self.bytes);
            if replaced.as_ref().err().is_none_or(|why| why.in_place) {
                disk.in_place = self.version;
            }
            replaced?;
            disk.durable = self.version;
        } else if disk.durable < self.version {
            crate::private::sync_parent(&self.file)
                .map_err(|e| Unreplaced { why: format!("{}: {e}", self.file.display()), in_place: true })?;
            disk.durable = disk.in_place;
        }
        Ok(self.written)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CreateError {
    BadName,
    TooMany,
    Unsaved(String),
}

/// A token id as the API spells it: 16 hex digits.
pub fn id_text(id: u64) -> String {
    format!("{id:016x}")
}

pub fn parse_id(text: &str) -> Option<u64> {
    (text.len() == 16 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        .then(|| u64::from_str_radix(text, 16).ok())
        .flatten()
}

/// Whether `name` may name a token or a profile: 1 to [`MAX_NAME`] characters, none of them a control character.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= MAX_NAME && !name.chars().any(char::is_control)
}

/// A random id that `taken` says no one has yet.
pub fn fresh_id(taken: impl Fn(u64) -> bool) -> u64 {
    loop {
        let id = rand::random();
        if !taken(id) {
            return id;
        }
    }
}

impl Tokens {
    /// No tokens (the demo's), never written.
    pub fn empty(state_dir: &Path) -> Tokens {
        Tokens { file: state_dir.join(FILE), tokens: Vec::new(), version: 0, disk: Arc::default() }
    }

    /// The stored tokens, none when there is no file yet. The file is read as a private file (see
    /// [`crate::private::read`]); one that cannot be read whole is refused: starting without it would forget
    /// revocations as well as tokens.
    pub fn load(state_dir: &Path) -> Result<Tokens, String> {
        let empty = Tokens::empty(state_dir);
        let file = &empty.file;
        let Some(bytes) = crate::private::read(file)? else { return Ok(empty) };
        let stored: File = crate::private::json(file, &bytes)?;
        let mut tokens = Vec::new();
        for s in stored.tokens {
            let (Some(id), Some(hash)) = (parse_id(&s.id), unhex(&s.sha256)) else {
                return Err(format!("{}: a malformed token entry", file.display()));
            };
            if !valid_name(&s.name) || tokens.iter().any(|t: &Token| t.id == id) {
                return Err(format!("{}: a malformed token entry", file.display()));
            }
            let (created, last_used) = (s.created, s.last_used);
            tokens.push(Token { id, name: s.name, hash, created, last_used, written_use: last_used, revoked: false });
        }
        if tokens.len() > MAX_TOKENS {
            return Err(format!("{}: more than {MAX_TOKENS} tokens", file.display()));
        }
        Ok(Tokens { tokens, ..empty })
    }

    /// Writes every token not revoked, now.
    fn save(&mut self) -> Result<(), Unreplaced> {
        let written = self.snapshot().write()?;
        self.written(&written);
        Ok(())
    }

    /// What a snapshot holds is on the file: the revoked tokens it left out go, and the last uses it holds are written
    /// (a use not written yet stays due).
    pub fn written(&mut self, written: &Written) {
        self.tokens.retain(|t| !(t.revoked && written.revoked.contains(&t.id)));
        for t in &mut self.tokens {
            if let Some((_, used)) = written.uses.iter().find(|(id, _)| *id == t.id) {
                t.written_use = t.written_use.max(*used);
            }
        }
    }

    /// Every token not revoked, as the file is to hold them.
    fn snapshot(&mut self) -> Snapshot {
        let tokens = self
            .tokens
            .iter()
            .filter(|t| !t.revoked)
            .map(|t| Stored {
                id: id_text(t.id),
                name: t.name.clone(),
                sha256: hex(&t.hash),
                created: t.created,
                last_used: t.last_used,
            })
            .collect();
        let bytes = serde_json::to_vec_pretty(&File { tokens }).expect("the tokens serialize");
        self.version += 1;
        let written = Written {
            revoked: self.tokens.iter().filter(|t| t.revoked).map(|t| t.id).collect(),
            uses: self.tokens.iter().map(|t| (t.id, t.last_used)).collect(),
        };
        Snapshot { file: self.file.clone(), bytes, version: self.version, disk: self.disk.clone(), written }
    }

    /// A snapshot to write when a revocation could not be written yet, or a token's last use is new to the file, or a
    /// minute newer than it.
    pub fn due(&mut self) -> Option<Snapshot> {
        let due = self.tokens.iter().any(|t| {
            t.revoked
                || t.last_used.is_some_and(|at| t.written_use.is_none_or(|w| at.saturating_sub(w) >= USE_WRITE_EVERY))
        });
        due.then(|| self.snapshot())
    }

    /// A new token named `name`: its listing, and the token itself, which is never shown again.
    pub fn create(&mut self, name: &str) -> Result<(Value, String), CreateError> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(CreateError::BadName);
        }
        if self.tokens.len() >= MAX_TOKENS {
            return Err(CreateError::TooMany);
        }
        let token = format!("{PREFIX}{}", crate::auth::random_token());
        let id = fresh_id(|id| self.tokens.iter().any(|t| t.id == id));
        let hash = sha256(&token);
        let entry = Token {
            id,
            name: name.to_owned(),
            hash,
            created: unix_now(),
            last_used: None,
            written_use: None,
            revoked: false,
        };
        self.tokens.push(entry);
        // In the file even when its directory could not be synced, so listed and counted at the next start: then kept,
        // and its token returned.
        if let Err(why) = crate::private::installed(self.save()) {
            self.tokens.pop();
            return Err(CreateError::Unsaved(why.into()));
        }
        let listed = self.describe(self.tokens.last().expect("just added"));
        Ok((listed, token))
    }

    fn describe(&self, t: &Token) -> Value {
        json!({"id": id_text(t.id), "name": t.name, "created": t.created, "last_used": t.last_used, "revoked": t.revoked})
    }

    /// Every token, never the token itself nor its hash.
    pub fn list(&self) -> Vec<Value> {
        self.tokens.iter().map(|t| self.describe(t)).collect()
    }

    /// The id of the live token `presented` is, comparing it with every stored one in constant time. Its use is
    /// recorded in memory; [`Tokens::due`] writes it.
    pub fn authenticate(&mut self, presented: &str) -> Option<u64> {
        let hash = sha256(presented);
        let mut found = None;
        for (i, t) in self.tokens.iter().enumerate() {
            if bool::from(t.hash.ct_eq(&hash)) && !t.revoked {
                found = Some(i);
            }
        }
        let token = &mut self.tokens[found?];
        token.last_used = Some(unix_now());
        Some(token.id)
    }

    /// Whether the token is still live: neither revoked nor gone.
    pub fn is_live(&self, id: u64) -> bool {
        self.tokens.iter().any(|t| t.id == id && !t.revoked)
    }

    /// Revokes a token: refused from now on, and written so before anything else happens. `None` when there is no
    /// such token; an error when the file could not be written durably: the token is refused all the same, listed as
    /// revoked and written out again (see [`Tokens::due`]) until that succeeds, but a restart before then may accept it
    /// again.
    pub fn revoke(&mut self, id: u64) -> Option<Result<(), String>> {
        let at = self.tokens.iter().position(|t| t.id == id)?;
        self.tokens[at].revoked = true;
        Some(self.save().map_err(String::from))
    }
}

/// The token an `Authorization: Bearer` header presents, if it has that shape.
pub fn presented(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    let well_formed = token.strip_prefix(PREFIX).is_some_and(|rest| {
        rest.len() == 43 && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    });
    (scheme.eq_ignore_ascii_case("bearer") && well_formed).then_some(token)
}

/// Why a bearer request is refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Denied {
    /// The demo has no tokens.
    Demo,
    /// A browser on another site sent it (an `Origin` that is not the admin's own).
    Origin,
    /// No live token.
    Unauthenticated,
}

/// The live token a bearer request presents. An `Origin` header, when present, must be the admin's own: tokens are for
/// agents, not for pages in a browser.
pub fn check(app: &crate::app::App, headers: &HeaderMap) -> Result<u64, Denied> {
    if app.config.demo.is_some() {
        return Err(Denied::Demo);
    }
    if headers.contains_key(header::ORIGIN) && !crate::auth::origin_matches(headers, &app.config.origin) {
        return Err(Denied::Origin);
    }
    presented(headers).and_then(|token| app.tokens.lock().unwrap().authenticate(token)).ok_or(Denied::Unauthenticated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bearer(token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        h
    }

    #[test]
    fn a_token_is_stored_as_its_hash_and_authenticates_after_a_restart() {
        let dir = crate::private::test_dir("tokens");
        let mut tokens = Tokens::load(&dir).unwrap();
        let (listed, token) = tokens.create("  laptop agent ").unwrap();
        assert_eq!(listed["name"], "laptop agent");
        assert!(token.starts_with("easi_") && token.len() == 48);
        let file = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(!file.contains(&token) && !file.contains(&token[5..]), "the token itself is never written");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join(FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(!tokens.list().iter().any(|t| t.to_string().contains(&token[5..])));
        let id = tokens.authenticate(presented(&bearer(&token)).unwrap()).expect("it authenticates");
        assert_eq!(id_text(id), listed["id"]);
        let written = tokens.due().expect("its first use is due").write().unwrap();
        tokens.written(&written);
        assert!(tokens.due().is_none(), "a use within the minute waits");
        let mut again = Tokens::load(&dir).unwrap();
        assert!(again.list()[0]["last_used"].is_u64(), "its first use was written");
        assert_eq!(again.authenticate(&token), Some(id));
        let mut other = token.clone();
        other.replace_range(10..11, if &token[10..11] == "A" { "B" } else { "A" });
        assert_eq!(again.authenticate(&other), None);
    }

    #[test]
    fn a_revoked_token_is_refused_and_written_out() {
        let dir = crate::private::test_dir("tokens");
        let mut tokens = Tokens::load(&dir).unwrap();
        let (_, first) = tokens.create("one").unwrap();
        let (_, second) = tokens.create("two").unwrap();
        let id = tokens.authenticate(&first).unwrap();
        assert_eq!(tokens.revoke(id), Some(Ok(())));
        assert!(!tokens.is_live(id));
        assert_eq!(tokens.authenticate(&first), None);
        assert_eq!(tokens.revoke(id), None, "already gone");
        let mut again = Tokens::load(&dir).unwrap();
        assert_eq!(again.authenticate(&first), None, "the revocation outlives a restart");
        assert!(again.authenticate(&second).is_some());
    }

    #[test]
    fn last_uses_written_late_never_bring_back_a_revoked_token() {
        let dir = crate::private::test_dir("tokens");
        let mut tokens = Tokens::load(&dir).unwrap();
        let (_, token) = tokens.create("one").unwrap();
        let id = tokens.authenticate(&token).unwrap();
        let uses = tokens.due().unwrap();
        assert_eq!(tokens.revoke(id), Some(Ok(())));
        uses.write().unwrap();
        assert!(Tokens::load(&dir).unwrap().list().is_empty(), "the revocation stays written");
    }

    /// Creates a token, uses it, and revokes it while its file cannot be written: its id, the token, and the file.
    fn revoke_unwritably(tokens: &mut Tokens, dir: &Path) -> (u64, String, PathBuf) {
        let (_, token) = tokens.create("one").unwrap();
        let id = tokens.authenticate(&token).unwrap();
        let good = tokens.file.clone();
        tokens.file = dir.join("missing").join(FILE);
        assert!(matches!(tokens.revoke(id), Some(Err(_))));
        (id, token, good)
    }

    #[test]
    fn a_revocation_that_cannot_be_written_still_refuses_the_token() {
        let dir = crate::private::test_dir("tokens");
        let mut tokens = Tokens::load(&dir).unwrap();
        let (id, token, good) = revoke_unwritably(&mut tokens, &dir);
        assert!(!tokens.is_live(id) && tokens.authenticate(&token).is_none());
        assert_eq!(tokens.list()[0]["revoked"], true, "listed as revoked until it can be written");
        // Written out again once the file can be written, then gone from memory too.
        let retry = tokens.due().expect("the revocation is due");
        assert!(retry.write().is_err(), "the file still cannot be written");
        tokens.file = good;
        let written = tokens.due().expect("due again").write().unwrap();
        tokens.written(&written);
        assert!(tokens.list().is_empty() && Tokens::load(&dir).unwrap().list().is_empty());
        assert!(tokens.due().is_none());
    }

    #[test]
    fn a_revocation_written_with_a_later_change_leaves_the_listing_and_the_count() {
        let dir = crate::private::test_dir("tokens");
        let mut tokens = Tokens::load(&dir).unwrap();
        let (_, _, good) = revoke_unwritably(&mut tokens, &dir);
        tokens.file = good;
        tokens.create("two").unwrap();
        assert_eq!(tokens.list().len(), 1, "only the new one: {:?}", tokens.list());
        assert_eq!(Tokens::load(&dir).unwrap().list().len(), 1);
    }

    #[test]
    fn a_revocation_a_later_snapshot_holds_stays_due_until_its_directory_is_synced() {
        let dir = crate::private::test_dir("tokens");
        let mut tokens = Tokens::load(&dir).unwrap();
        let (_, _, good) = revoke_unwritably(&mut tokens, &dir);
        let unsyncable = tokens.due().expect("the revocation is due");
        tokens.file = good;
        let older = tokens.due().expect("due again");
        tokens.due().expect("due again").write().unwrap();
        // As if that later snapshot's directory could not be synced.
        tokens.disk.lock().unwrap().durable = 0;
        assert!(unsyncable.write().is_err(), "the later snapshot may not outlive a power loss");
        assert_eq!(tokens.list()[0]["revoked"], true, "still due");
        let written = older.write().unwrap();
        tokens.written(&written);
        assert!(tokens.list().is_empty() && Tokens::load(&dir).unwrap().list().is_empty());
    }

    #[test]
    fn a_last_use_that_cannot_be_written_stays_due() {
        let dir = crate::private::test_dir("tokens");
        let mut tokens = Tokens::load(&dir).unwrap();
        let (_, token) = tokens.create("one").unwrap();
        tokens.authenticate(&token).unwrap();
        let good = tokens.file.clone();
        tokens.file = dir.join("missing").join(FILE);
        assert!(tokens.due().expect("its first use is due").write().is_err());
        tokens.file = good;
        let written = tokens.due().expect("still due").write().unwrap();
        tokens.written(&written);
        assert!(tokens.due().is_none());
        assert!(Tokens::load(&dir).unwrap().list()[0]["last_used"].is_u64());
    }

    #[test]
    fn names_and_counts_are_bounded() {
        let dir = crate::private::test_dir("tokens");
        let mut tokens = Tokens::load(&dir).unwrap();
        assert_eq!(tokens.create("  ").unwrap_err(), CreateError::BadName);
        assert_eq!(tokens.create(&"x".repeat(MAX_NAME + 1)).unwrap_err(), CreateError::BadName);
        assert_eq!(tokens.create("a\nb").unwrap_err(), CreateError::BadName);
        for i in 0..MAX_TOKENS {
            tokens.create(&format!("t{i}")).unwrap();
        }
        assert_eq!(tokens.create("one more").unwrap_err(), CreateError::TooMany);
    }

    #[test]
    fn a_damaged_file_is_refused_at_startup() {
        let dir = crate::private::test_dir("tokens");
        crate::private::replace(&dir.join(FILE), b"{\"tokens\": [{\"id\": \"zz\"}]}").unwrap();
        let refused = Tokens::load(&dir).err().expect("refused");
        assert!(refused.contains("not the expected JSON"), "refused for its contents: {refused}");
    }

    #[test]
    fn only_a_well_formed_bearer_header_presents_a_token() {
        let token = format!("easi_{}", "A".repeat(43));
        assert_eq!(presented(&bearer(&token)), Some(token.as_str()));
        let mut lower = HeaderMap::new();
        lower.insert(header::AUTHORIZATION, format!("bearer {token}").parse().unwrap());
        assert_eq!(presented(&lower), Some(token.as_str()));
        for bad in [format!("Basic {token}"), "Bearer easi_short".into(), format!("Bearer {}", &token[5..])] {
            let mut h = HeaderMap::new();
            h.insert(header::AUTHORIZATION, bad.parse().unwrap());
            assert_eq!(presented(&h), None, "{bad}");
        }
        assert_eq!(presented(&HeaderMap::new()), None);
    }
}
