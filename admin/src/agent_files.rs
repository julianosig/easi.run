//! The agent's files, while it holds control: what the browser downloads, files the agent sends into the browser, and
//! the page's file chooser.
//!
//! Each running session outside the demo keeps one link to its display (`Link::AgentFiles`). While no person holds
//! control, the display sends it what it would send the holder: download offers, the page's choosers, the answers to
//! uploads; the link sends only the file messages a holder would, and the display refuses any other input on it. A
//! lease change ends everything of the previous lease, here as on the display.
//!
//! The administrator's permissions (`/api/agent-files`, kept in `agent-files.json`) say what the agent may do: save a
//! download (`downloads`) and send a file into the browser (`transfers`), each `ask` (the default), `allow` or `deny`;
//! and give the files it sent to the page's file chooser (`uploads`), `allow` or `deny` (the default). A session's
//! own features apply as well: `downloads` for the first, `uploads` for the others. An ask waits up to [`ASK_FOR`] for
//! the administrator's answer on the session's page. The bytes go by single-use URLs: a 256-bit ticket bound to the
//! session, the caller and the agent's lease epoch, which the agent fetches (a download) or puts the file to, so an
//! agent without a token (the local socket) can use them too.

use std::collections::HashMap;
use std::path::{Path as FilePath, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use display_protocol::frame::{FrameReader, Link, MAX_CHUNK};
use display_protocol::{Direction, FILE_CHUNK, FILE_WINDOW, MAX_CHOSEN, MAX_FILE_NAME, MAX_UPLOAD, Message};
use display_protocol::{UPLOAD_CHUNK, UPLOAD_WINDOW};
use futures_util::StreamExt;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot, watch};

use crate::api::{Shared, error};
use crate::app::{App, Owner, Piece, Session};
use crate::coded::Code;
use crate::service::Called;

/// How long an ask waits for the administrator's answer.
pub const ASK_FOR: Duration = Duration::from_secs(120);
/// How long a file URL may wait to be used.
const TICKET_FOR: Duration = Duration::from_secs(300);
/// Asks waiting at once, and unused file URLs, per session; past the URLs' bound the oldest goes.
const MAX_ASKS: usize = 4;
const MAX_TICKETS: usize = 8;
/// Files the agent sent that one lease keeps, well within what the display keeps for a holder (so it never forgets one
/// silently).
const MAX_SENT: usize = 32;
/// Downloads remembered as offered, as many as the display keeps.
const MAX_OFFERS: usize = 16;
/// Pieces of a download waiting for its response: the display sends at most [`FILE_WINDOW`] past the credit.
const PIECES: usize = (FILE_WINDOW / FILE_CHUNK as u64) as usize + 2;
/// Credit is granted again once the response took this much more.
const CREDIT_EVERY: u64 = FILE_WINDOW / 4;
/// How long the browser may take to store a file once all its bytes went.
const STORE_WITHIN: Duration = Duration::from_secs(60);
const LINK_QUEUE: usize = 64;

/// One permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Choice {
    Ask,
    Allow,
    Deny,
}

/// What the agent may do with files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Saving what the browser downloads.
    pub downloads: Choice,
    /// Sending its own files into the browser.
    pub transfers: Choice,
    /// Giving the files it sent to the page's file chooser: `allow` or `deny`.
    pub uploads: Choice,
}

impl Default for Policy {
    fn default() -> Policy {
        Policy { downloads: Choice::Ask, transfers: Choice::Ask, uploads: Choice::Deny }
    }
}

impl Policy {
    pub fn valid(&self) -> bool {
        self.uploads != Choice::Ask
    }
}

/// The permissions and their file.
pub struct Policies {
    file: PathBuf,
    pub policy: Policy,
}

impl Policies {
    /// The stored permissions, the defaults when there is no file yet. The file is read as a private file (see
    /// [`crate::private::read`]).
    pub fn load(state_dir: &FilePath) -> Result<Policies, String> {
        let file = state_dir.join("agent-files.json");
        let policy = match crate::private::read(&file)? {
            None => Policy::default(),
            Some(bytes) => crate::private::json::<Policy>(&file, &bytes)?,
        };
        if !policy.valid() {
            return Err(format!("{}: `uploads` is `allow` or `deny`", file.display()));
        }
        Ok(Policies { file, policy })
    }

    /// The defaults, never written (the demo's).
    pub fn empty(state_dir: &FilePath) -> Policies {
        Policies { file: state_dir.join("agent-files.json"), policy: Policy::default() }
    }

    /// Writes `policy` and takes it, or keeps the previous one when it could not be written.
    pub fn set(&mut self, policy: Policy) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&policy).expect("a policy encodes");
        crate::private::installed(crate::private::replace(&self.file, &bytes)).map_err(String::from)?;
        self.policy = policy;
        Ok(())
    }
}

struct Offer {
    file: u32,
    name: String,
    size: Option<u64>,
}

/// A file the agent sent into the browser, by the display's upload number.
struct Sent {
    upload: u32,
    name: String,
    size: u64,
    stored: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Download,
    Transfer,
}

/// A question for the administrator.
struct Ask {
    id: u32,
    kind: Kind,
    name: String,
    size: Option<u64>,
    answer: oneshot::Sender<bool>,
}

enum Use {
    Download { file: u32, name: String, size: Option<u64> },
    Send { name: String, size: u64 },
}

struct Ticket {
    key: [u8; 32],
    who: Owner,
    epoch: u32,
    until: Instant,
    what: Use,
}

#[derive(Clone, Copy, Default)]
struct Progress {
    acked: u64,
    done: Option<bool>,
}

/// One session's files, as its link knows them.
#[derive(Default)]
pub struct Files {
    /// The link's queues: one for the bytes of uploads and download credit, which wait for room; one for the rest
    /// (answers to choosers, cancellations), which never waits behind them.
    link: Option<mpsc::Sender<Link>>,
    control: Option<mpsc::Sender<Link>>,
    /// The agent's lease epoch they belong to, while it holds control.
    epoch: Option<u32>,
    offers: Vec<Offer>,
    /// The page's open chooser: its request, and whether it takes several files.
    chooser: Option<(u32, bool)>,
    sent: Vec<Sent>,
    next_upload: u32,
    asks: Vec<Ask>,
    next_ask: u32,
    tickets: Vec<Ticket>,
    receiving: HashMap<u32, mpsc::Sender<Piece>>,
    sending: HashMap<u32, watch::Sender<Progress>>,
}

impl Files {
    /// Starts over when the lease changed: everything of the previous one ends (an ask unanswered is refused, a
    /// transfer fails).
    fn at(&mut self, epoch: Option<u32>) {
        if self.epoch == epoch {
            return;
        }
        let (link, control) = (self.link.take(), self.control.take());
        let (next_upload, next_ask) = (self.next_upload, self.next_ask);
        *self = Files { link, control, epoch, next_upload, next_ask, ..Files::default() };
    }

    /// Sends a control message; false when the link is gone or its queue full.
    fn send(&self, message: Message) -> bool {
        self.control.as_ref().is_some_and(|control| control.try_send(Link::Relay(message)).is_ok())
    }
}

/// The agent's lease epoch now, while it holds control.
fn agent_epoch(session: &Session) -> Option<u32> {
    *session.state.lock().unwrap().files_lease.borrow()
}

/// Opens the session's files link and keeps it until the session ends.
pub fn spawn(app: Shared, session: Arc<Session>) {
    tokio::spawn(async move {
        let outcome = link(&app, &session).await;
        let ended = *session.ended.borrow();
        if let Err(why) = outcome
            && !ended
        {
            tracing::warn!(session = %session.id, %why, "the agent's files link ended");
        }
        let mut files = session.files.lock().unwrap();
        (files.link, files.control) = (None, None);
        files.at(None);
    });
}

async fn link(app: &App, session: &Session) -> Result<(), String> {
    let path = app.config.streams_dir.join(&session.id).join("display.sock");
    let stream = UnixStream::connect(&path).await.map_err(|e| format!("display: {e}"))?;
    let (mut read, mut write) = stream.into_split();
    write.write_all(&Link::AgentFiles.encode()).await.map_err(|e| format!("display: {e}"))?;
    let (tx, mut rx) = mpsc::channel(LINK_QUEUE);
    let (control, mut controls) = mpsc::channel(LINK_QUEUE);
    {
        let mut files = session.files.lock().unwrap();
        (files.link, files.control) = (Some(tx), Some(control));
    }
    let mut lease = session.state.lock().unwrap().files_lease.subscribe();
    lease.mark_changed();
    let mut ended = session.ended.subscribe();
    let mut frames = FrameReader::new(Direction::ServerToClient);
    let mut chunk = vec![0u8; MAX_CHUNK];
    loop {
        tokio::select! {
            biased;
            _ = async { drop(ended.wait_for(|ended| *ended).await) } => return Ok(()),
            Some(link) = controls.recv() => write.write_all(&link.encode()).await.map_err(|e| format!("display: {e}"))?,
            changed = lease.changed() => {
                changed.map_err(|_| "the session went")?;
                let epoch = *lease.borrow_and_update();
                session.files.lock().unwrap().at(epoch);
                // A grant only while the agent holds control: never one that could race a holder's of the same epoch.
                if let Some(lease_epoch) = epoch {
                    let grant = Link::Grant { lease_epoch, holder: false }.encode();
                    write.write_all(&grant).await.map_err(|e| format!("display: {e}"))?;
                }
            }
            Some(link) = rx.recv() => write.write_all(&link.encode()).await.map_err(|e| format!("display: {e}"))?,
            n = read.read(&mut chunk) => {
                let n = n.map_err(|e| format!("display: {e}"))?;
                if n == 0 {
                    return Err("the display closed the link".into());
                }
                frames.push(&chunk[..n]).map_err(|e| format!("display: {e:?}"))?;
                while let Some(body) = frames.next_body().map_err(|e| format!("display: {e:?}"))? {
                    if let Ok(Link::Relay(message)) = Link::decode(&body, Direction::ServerToClient) {
                        heard(app, session, message);
                    }
                }
            }
        }
    }
}

/// What the display sent the agent's link.
fn heard(app: &App, session: &Session, message: Message) {
    let epoch = agent_epoch(session);
    let mut files = session.files.lock().unwrap();
    // A message of the new lease may come before its change is seen here.
    files.at(epoch);
    let Some(epoch) = epoch.filter(|&e| message.lease_epoch() == Some(e)) else { return };
    match message {
        Message::FileOffer { file, size, name, .. } => {
            files.offers.retain(|o| o.file != file);
            if files.offers.len() >= MAX_OFFERS {
                files.offers.remove(0);
            }
            files.offers.push(Offer { file, name, size });
        }
        Message::FileData { file, data, .. } => {
            let taken = files.receiving.get(&file).is_some_and(|tx| tx.try_send(Piece::Data(data.into())).is_ok());
            if !taken && files.receiving.remove(&file).is_some() {
                files.send(Message::FileCancel { lease_epoch: epoch, file });
            }
        }
        Message::FileEnd { file, .. } => {
            if let Some(tx) = files.receiving.remove(&file) {
                let _ = tx.try_send(Piece::End);
            }
            files.offers.retain(|o| o.file != file);
        }
        Message::FileGone { file, .. } => {
            files.receiving.remove(&file);
            files.offers.retain(|o| o.file != file);
        }
        Message::FileRequest { request, multiple, .. } => {
            let allowed = app.agent_files.lock().unwrap().policy.uploads == Choice::Allow && session.features.uploads;
            if allowed {
                files.chooser = Some((request, multiple));
            } else if !files.send(Message::FileChoose { lease_epoch: epoch, request, uploads: Vec::new() }) {
                // Nothing would answer it: the page hears at once that no file was chosen, or, when even that cannot
                // go, at the next lease change.
                tracing::warn!(session = %session.id, "the page's file chooser could not be cancelled now");
            }
        }
        Message::FileRequestEnd { request, .. } => {
            if files.chooser.is_some_and(|(r, _)| r == request) {
                files.chooser = None;
            }
        }
        Message::UploadAck { upload, received, .. } => {
            if let Some(progress) = files.sending.get(&upload) {
                progress.send_modify(|p| p.acked = p.acked.max(received));
            }
        }
        Message::UploadDone { upload, stored, .. } => {
            if let Some(progress) = files.sending.remove(&upload) {
                progress.send_modify(|p| p.done = Some(stored));
            }
            if stored {
                files.sent.iter_mut().filter(|s| s.upload == upload).for_each(|s| s.stored = true);
            } else {
                files.sent.retain(|s| s.upload != upload);
            }
        }
        _ => {}
    }
}

const NAMES: [&str; 4] = ["list_files", "save_download", "send_file", "choose_files"];

pub fn known(name: &str) -> bool {
    NAMES.contains(&name)
}

static TOOLS: LazyLock<Vec<Value>> = LazyLock::new(|| {
    let read_only =
        json!({"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false});
    let acts = json!({"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": true});
    vec![
        json!({
            "name": "list_files",
            "description": "The files of this session while you hold control: `downloads`, what the browser downloaded and waits \
                for you (save one with save_download); `files`, those you sent into the browser (send_file); `chooser`, the \
                file chooser the page opened, if one is open (answer it with choose_files); and `permissions`, what the \
                administrator lets you do (ask: they are asked first; deny: refused). After clicking a page's upload button, \
                call this to see its chooser.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": read_only,
        }),
        json!({
            "name": "save_download",
            "description": "Gives you a download the browser made, by its id from list_files: a URL to GET once, within 5 \
                minutes, for its exact bytes (for example `curl -fo <name> '<url>'`). When the permission is `ask`, this \
                waits up to 2 minutes for the administrator to allow it on the session's page.",
            "inputSchema": {
                "type": "object",
                "properties": {"download": {"type": "integer", "minimum": 1, "maximum": u32::MAX}},
                "required": ["download"],
                "additionalProperties": false,
            },
            "annotations": acts,
        }),
        json!({
            "name": "send_file",
            "description": "Sends one of your own files into the browser, for the page's file chooser (choose_files): a URL \
                to PUT the file's exact bytes to once, within 5 minutes (for example `curl -fT <file> '<url>'`), named and \
                sized as given here. Its answer gives the file's id. When the permission is `ask`, this waits up to 2 \
                minutes for the administrator to allow it on the session's page.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "minLength": 1, "maxLength": MAX_FILE_NAME},
                    "size": {"type": "integer", "minimum": 0, "maximum": MAX_UPLOAD},
                },
                "required": ["name", "size"],
                "additionalProperties": false,
            },
            "annotations": acts,
        }),
        json!({
            "name": "choose_files",
            "description": "Answers the file chooser the page opened (its `request` from list_files) with files you sent \
                (their ids); none cancels it. A chooser that takes one file takes at most one.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "request": {"type": "integer", "minimum": 1, "maximum": u32::MAX},
                    "files": {
                        "type": "array",
                        "items": {"type": "integer", "minimum": 1, "maximum": u32::MAX},
                        "maxItems": MAX_CHOSEN,
                    },
                },
                "required": ["request", "files"],
                "additionalProperties": false,
            },
            "annotations": acts,
        }),
    ]
});

/// Each tool as MCP lists it.
pub fn tools() -> &'static [Value] {
    &TOOLS
}

/// A tool's MCP result.
fn result(value: Value) -> Called {
    Called::Done(
        json!({"content": [{"type": "text", "text": value.to_string()}], "structuredContent": value, "isError": false}),
    )
}

fn failed(why: impl Into<String>) -> Called {
    Called::Failed(why.into())
}

/// Runs one of these tools for `who` on `session`, while the agent holds control. The call holds one of the agent's
/// places throughout, an ask's wait included, so the agent counts as at work: a person who opens the session meanwhile
/// watches, and can answer.
pub async fn call(app: &App, who: Owner, session: &Session, tool: &str, arguments: Value) -> Called {
    if app.config.demo.is_some() {
        return failed("not in the demo");
    }
    let Some(_turn) = session.agent.place() else { return Called::Busy };
    if session.agent.is_blocked() {
        return Called::Controlled;
    }
    let Some(epoch) = agent_epoch(session) else { return Called::Controlled };
    let number = |key: &str| arguments.get(key).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
    let outcome = match tool {
        "list_files" => Ok(listed(app, session)),
        "save_download" => match number("download") {
            Some(file) => save_download(app, who, session, epoch, file).await,
            None => Err("`download` is a download's id from list_files".into()),
        },
        "send_file" => {
            let name = arguments.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
            match arguments.get("size").and_then(Value::as_u64) {
                Some(size) => send_file(app, who, session, epoch, name, size).await,
                None => Err("`size` is the file's size in bytes".into()),
            }
        }
        "choose_files" => {
            let files: Option<Vec<u32>> = arguments
                .get("files")
                .and_then(Value::as_array)
                .and_then(|ids| ids.iter().map(|id| id.as_u64().and_then(|n| u32::try_from(n).ok())).collect());
            match (number("request"), files) {
                (Some(request), Some(files)) => choose(app, session, epoch, request, files),
                _ => Err("`request` is the chooser's from list_files, and `files` the ids of files you sent".into()),
            }
        }
        _ => Err("no such tool".into()),
    };
    if !app.may_act(who) {
        return Called::Revoked;
    }
    if !app.reachable(who, session) {
        return Called::Unreached;
    }
    match outcome {
        Ok(value) => match crate::service::redacted(session, value, None).await {
            Some((value, _)) => result(value),
            None => failed("the answer would take more memory than is free once redacted"),
        },
        Err(why) => failed(why),
    }
}

fn listed(app: &App, session: &Session) -> Value {
    let policy = app.agent_files.lock().unwrap().policy;
    let files = session.files.lock().unwrap();
    json!({
        "downloads": files.offers.iter().map(|o| json!({"id": o.file, "name": o.name, "size": o.size})).collect::<Vec<_>>(),
        "files": files.sent.iter().filter(|s| s.stored).map(|s| json!({"id": s.upload, "name": s.name, "size": s.size})).collect::<Vec<_>>(),
        "chooser": files.chooser.map(|(request, multiple)| json!({"request": request, "multiple": multiple})),
        "permissions": policy,
    })
}

/// The administrator's answer to an ask, or why there is none.
async fn asked(session: &Session, epoch: u32, kind: Kind, name: &str, size: Option<u64>) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    let id = {
        let mut files = session.files.lock().unwrap();
        if files.epoch != Some(epoch) {
            return Err("control moved".into());
        }
        if files.asks.len() >= MAX_ASKS {
            return Err("too many questions wait for the administrator already".into());
        }
        files.next_ask = files.next_ask.wrapping_add(1).max(1);
        let id = files.next_ask;
        files.asks.push(Ask { id, kind, name: name.to_owned(), size, answer: tx });
        id
    };
    // Gone with the wait, however it ends: a caller that leaves takes its question with it.
    struct Asking<'a>(&'a Session, u32);
    impl Drop for Asking<'_> {
        fn drop(&mut self) {
            self.0.files.lock().unwrap().asks.retain(|a| a.id != self.1);
        }
    }
    let asking = Asking(session, id);
    let answer = tokio::time::timeout(ASK_FOR, rx).await;
    drop(asking);
    match answer {
        Ok(Ok(true)) => Ok(()),
        Ok(Ok(false)) => Err("the administrator refused".into()),
        Ok(Err(_)) => Err("control moved before the administrator answered".into()),
        Err(_) => Err(format!(
            "nobody answered within {} s: ask the person to open the session's watch_url to answer, then try again",
            ASK_FOR.as_secs()
        )),
    }
}

/// The permission `choice`, asked when it says so.
async fn permitted(
    session: &Session,
    epoch: u32,
    choice: Choice,
    kind: Kind,
    name: &str,
    size: Option<u64>,
) -> Result<(), String> {
    match choice {
        Choice::Allow => Ok(()),
        Choice::Deny => Err(match kind {
            Kind::Download => "the administrator does not let agents save downloads".into(),
            Kind::Transfer => "the administrator does not let agents send files into the browser".into(),
        }),
        Choice::Ask => asked(session, epoch, kind, name, size).await,
    }
}

/// A new single-use URL for `what`.
fn ticket(app: &App, who: Owner, session: &Session, epoch: u32, what: Use) -> Result<String, String> {
    let mut key = [0u8; 32];
    rand::rng().fill_bytes(&mut key);
    let mut files = session.files.lock().unwrap();
    if files.epoch != Some(epoch) {
        return Err("control moved".into());
    }
    files.tickets.retain(|t| t.until > Instant::now());
    if files.tickets.len() >= MAX_TICKETS {
        files.tickets.remove(0);
    }
    files.tickets.push(Ticket { key, who, epoch, until: Instant::now() + TICKET_FOR, what });
    let key = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key);
    Ok(format!("{}/api/agent/sessions/{}/files/{key}", app.config.origin, session.id))
}

async fn save_download(app: &App, who: Owner, session: &Session, epoch: u32, file: u32) -> Result<Value, String> {
    if !session.features.downloads {
        return Err("this session's downloads are off".into());
    }
    let offered = |session: &Session| {
        let files = session.files.lock().unwrap();
        files.offers.iter().find(|o| o.file == file).map(|o| (o.name.clone(), o.size))
    };
    let Some((name, size)) = offered(session) else {
        return Err("no such download: list_files lists the downloads waiting".into());
    };
    let choice = app.agent_files.lock().unwrap().policy.downloads;
    permitted(session, epoch, choice, Kind::Download, &name, size).await?;
    if offered(session).is_none() {
        return Err("the download is no longer there".into());
    }
    let url = ticket(app, who, session, epoch, Use::Download { file, name: name.clone(), size })?;
    Ok(json!({"url": url, "name": name, "size": size, "expires_in_s": TICKET_FOR.as_secs()}))
}

/// A name the browser may store a file under: a plain name, not a path.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_FILE_NAME
        && name != "."
        && name != ".."
        && !name.chars().any(|c| c == '/' || c == '\\' || c.is_control())
}

async fn send_file(
    app: &App,
    who: Owner,
    session: &Session,
    epoch: u32,
    name: String,
    size: u64,
) -> Result<Value, String> {
    if !session.features.uploads {
        return Err("this session's uploads are off".into());
    }
    if !plain_name(&name) {
        return Err(format!("`name` is a file's name, not a path, of 1 to {MAX_FILE_NAME} bytes"));
    }
    if size > MAX_UPLOAD {
        return Err(format!("a file is at most {MAX_UPLOAD} bytes"));
    }
    if session.files.lock().unwrap().sent.len() >= MAX_SENT {
        return Err(format!("at most {MAX_SENT} files go into the browser while you hold control"));
    }
    let choice = app.agent_files.lock().unwrap().policy.transfers;
    permitted(session, epoch, choice, Kind::Transfer, &name, Some(size)).await?;
    let url = ticket(app, who, session, epoch, Use::Send { name, size })?;
    Ok(json!({"url": url, "expires_in_s": TICKET_FOR.as_secs()}))
}

fn choose(app: &App, session: &Session, epoch: u32, request: u32, chosen: Vec<u32>) -> Result<Value, String> {
    if app.agent_files.lock().unwrap().policy.uploads != Choice::Allow || !session.features.uploads {
        return Err("the administrator does not let agents give files to the page".into());
    }
    let mut files = session.files.lock().unwrap();
    let Some((open, multiple)) = files.chooser.filter(|_| files.epoch == Some(epoch)) else {
        return Err("no file chooser is open: click the page's upload button, then list_files".into());
    };
    if open != request {
        return Err("that chooser closed; list_files shows the one open now".into());
    }
    if chosen.len() > if multiple { MAX_CHOSEN } else { 1 } {
        return Err("this chooser takes one file".into());
    }
    if !chosen.iter().all(|id| files.sent.iter().any(|s| s.upload == *id && s.stored)) {
        return Err("no such file: list_files lists the files you sent".into());
    }
    let count = chosen.len();
    if !files.send(Message::FileChoose { lease_epoch: epoch, request, uploads: chosen }) {
        return Err("the session is busy; try again".into());
    }
    files.chooser = None;
    Ok(json!({"chosen": count}))
}

/// What the administrator's page shows: the session's open asks.
pub fn asks(session: &Session) -> Value {
    let files = session.files.lock().unwrap();
    let asks: Vec<Value> =
        files.asks.iter().map(|a| json!({"id": a.id, "kind": a.kind, "name": a.name, "size": a.size})).collect();
    json!({ "asks": asks })
}

/// The administrator's answer to ask `id`; false when it is not open.
pub fn answer(session: &Session, id: u32, allow: bool) -> bool {
    let mut files = session.files.lock().unwrap();
    let Some(index) = files.asks.iter().position(|a| a.id == id) else { return false };
    files.asks.remove(index).answer.send(allow).is_ok()
}

fn refused(status: StatusCode, code: Code, why: &str) -> Response {
    error(status, code, why)
}

/// The ticket `key` of session `id`, taken out (single use), if it is still good: unexpired, of the agent's lease now,
/// and of a caller that may still act on the session.
fn take(app: &App, id: &str, key: &str) -> Result<(Arc<Session>, Ticket), Box<Response>> {
    let none = || Box::new(refused(StatusCode::NOT_FOUND, Code::NoDownload, "no such file URL"));
    let key: [u8; 32] = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(key)
        .ok()
        .and_then(|k| k.try_into().ok())
        .ok_or_else(none)?;
    let session = app.session(id).ok_or_else(none)?;
    let ticket = {
        let mut files = session.files.lock().unwrap();
        let index = files.tickets.iter().position(|t| t.key == key && t.until > Instant::now()).ok_or_else(none)?;
        files.tickets.remove(index)
    };
    if agent_epoch(&session) != Some(ticket.epoch) {
        return Err(Box::new(refused(StatusCode::GONE, Code::ControlMoved, "control moved")));
    }
    if !(app.may_act(ticket.who) && app.reachable(ticket.who, &session)) {
        return Err(none());
    }
    Ok((session, ticket))
}

/// `GET` of a download's URL: its exact bytes, streamed as the browser writes them.
pub async fn fetch(State(app): State<Shared>, Path((id, key)): Path<(String, String)>) -> Response {
    let (session, ticket) = match take(&app, &id, &key) {
        Ok(taken) => taken,
        Err(response) => return *response,
    };
    let Use::Download { file, name, size } = ticket.what else {
        return refused(StatusCode::NOT_FOUND, Code::NoDownload, "no such file URL");
    };
    let Some(slot) = crate::app::Place::take(app.downloads.clone(), crate::api::MAX_DOWNLOADS) else {
        return refused(StatusCode::SERVICE_UNAVAILABLE, Code::DownloadsBusy, "too many downloads at once");
    };
    let (tx, rx) = mpsc::channel(PIECES);
    let link = {
        let mut files = session.files.lock().unwrap();
        let there = files.epoch == Some(ticket.epoch) && files.offers.iter().any(|o| o.file == file);
        let (true, Some(link)) = (there, files.link.clone()) else {
            return refused(StatusCode::GONE, Code::DownloadGone, "the download is no longer available");
        };
        // One response per download: another URL of it waits for this one to end.
        if files.receiving.contains_key(&file) {
            return refused(StatusCode::CONFLICT, Code::DownloadsBusy, "this download is being received already");
        }
        files.receiving.insert(file, tx);
        link
    };
    let epoch = ticket.epoch;
    // Its guard first: a response dropped while the first credit waits for room still lets go of the download.
    let receiving = Receiving { session, file, epoch, complete: false };
    let _ = link.send(Link::Relay(Message::FileRead { lease_epoch: epoch, file, offset: 0 })).await;
    let state = (receiving, rx, link, ticket.who, app.clone(), size, 0u64, 0u64, None::<Bytes>, slot);
    let body = futures_util::stream::unfold(Some(state), |state| async move {
        let ended = || std::io::Error::other("the download ended before it was complete");
        let (mut receiving, mut rx, link, who, app, size, mut taken, mut granted, mut held, slot) = state?;
        // Checked before each wait and again before anything goes out: a caller who may no longer act gets nothing more.
        let still = |app: &App, receiving: &Receiving| {
            app.may_act(who)
                && app.reachable(who, &receiving.session)
                && agent_epoch(&receiving.session) == Some(receiving.epoch)
        };
        loop {
            let piece = if still(&app, &receiving) { rx.recv().await } else { None };
            let piece = piece.filter(|_| still(&app, &receiving));
            match piece {
                Some(Piece::Data(bytes)) => {
                    taken += bytes.len() as u64;
                    if size.is_some_and(|size| taken > size) {
                        return Some((Err(ended()), None));
                    }
                    if taken - granted >= CREDIT_EVERY {
                        granted = taken;
                        let read =
                            Message::FileRead { lease_epoch: receiving.epoch, file: receiving.file, offset: taken };
                        let _ = link.send(Link::Relay(read)).await;
                        if !still(&app, &receiving) {
                            return Some((Err(ended()), None));
                        }
                    }
                    // The last piece waits for the confirmed end, so a whole response is always the whole file.
                    if let Some(out) = held.replace(bytes) {
                        let state = (receiving, rx, link, who, app, size, taken, granted, held, slot);
                        return Some((Ok(out), Some(state)));
                    }
                }
                Some(Piece::End) if size.is_none_or(|size| taken == size) => {
                    receiving.complete = true;
                    return held.take().map(|out| (Ok(out), None));
                }
                _ => return Some((Err(ended()), None)),
            }
        }
    });
    let mut response = Body::from_stream(body).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(size) = size.filter(|&size| size > 0) {
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from(size));
    }
    if let Ok(value) = HeaderValue::from_str(&crate::api::content_disposition(&name)) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

/// A download being received for the agent: one that ends before it is complete is cancelled on the display.
struct Receiving {
    session: Arc<Session>,
    file: u32,
    epoch: u32,
    complete: bool,
}

impl Drop for Receiving {
    fn drop(&mut self) {
        let mut files = self.session.files.lock().unwrap();
        files.receiving.remove(&self.file);
        if !self.complete && files.epoch == Some(self.epoch) {
            files.send(Message::FileCancel { lease_epoch: self.epoch, file: self.file });
        }
    }
}

/// `PUT` of a file's URL: its bytes go into the browser, within the window the display acknowledges; answered once the
/// browser stored it, with its id for `choose_files`.
pub async fn put(
    State(app): State<Shared>,
    Path((id, key)): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let (session, ticket) = match take(&app, &id, &key) {
        Ok(taken) => taken,
        Err(response) => return *response,
    };
    let Use::Send { name, size } = ticket.what else {
        return refused(StatusCode::NOT_FOUND, Code::NoDownload, "no such file URL");
    };
    let length = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    if length != Some(size) {
        let why = format!("the body is exactly the file send_file named: {size} bytes, with its Content-Length");
        return refused(StatusCode::BAD_REQUEST, Code::BadRequest, &why);
    }
    let epoch = ticket.epoch;
    let (progress, mut acked) = watch::channel(Progress::default());
    let (upload, link) = {
        let mut files = session.files.lock().unwrap();
        let link = files.link.clone().filter(|_| files.epoch == Some(epoch) && files.sent.len() < MAX_SENT);
        let Some(link) = link else {
            return refused(StatusCode::GONE, Code::ControlMoved, "control moved, or too many files were sent");
        };
        files.next_upload = files.next_upload.wrapping_add(1).max(1);
        let upload = files.next_upload;
        files.sent.push(Sent { upload, name: name.clone(), size, stored: false });
        files.sending.insert(upload, progress);
        (upload, link)
    };
    let sent = async {
        let start = Message::UploadStart { lease_epoch: epoch, upload, size, name: name.clone() };
        link.send(Link::Relay(start)).await.map_err(|_| "the session's link closed")?;
        let mut offset = 0u64;
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "the request's body was cut")?;
            for piece in chunk.chunks(UPLOAD_CHUNK) {
                let end = offset + piece.len() as u64;
                if end > size {
                    return Err("the body is longer than its size");
                }
                loop {
                    let now = *acked.borrow_and_update();
                    if now.done.is_some() {
                        return Err("the browser refused the file");
                    }
                    if end <= now.acked + UPLOAD_WINDOW {
                        break;
                    }
                    acked.changed().await.map_err(|_| "control moved")?;
                }
                let reached = app.may_act(ticket.who) && app.reachable(ticket.who, &session);
                if !reached || agent_epoch(&session) != Some(epoch) {
                    return Err("control moved");
                }
                let data = Message::UploadData { lease_epoch: epoch, upload, offset, data: piece.to_vec() };
                link.send(Link::Relay(data)).await.map_err(|_| "the session's link closed")?;
                offset = end;
            }
        }
        if offset != size {
            return Err("the body is shorter than its size");
        }
        let stored = tokio::time::timeout(STORE_WITHIN, acked.wait_for(|p| p.done.is_some()))
            .await
            .map_err(|_| "the browser did not store the file in time")?
            .map_err(|_| "control moved")?
            .done;
        if stored == Some(true) { Ok(()) } else { Err("the browser could not store the file") }
    };
    match sent.await {
        Ok(()) => axum::Json(json!({"file": upload, "name": name, "size": size})).into_response(),
        Err(why) => {
            let mut files = session.files.lock().unwrap();
            files.sent.retain(|s| s.upload != upload);
            files.sending.remove(&upload);
            refused(StatusCode::CONFLICT, Code::BadRequest, why)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_permissions_start_at_the_safest_and_are_kept() {
        let dir = crate::private::test_dir("agent-files");
        let mut policies = Policies::load(&dir).unwrap();
        assert_eq!(policies.policy, Policy { downloads: Choice::Ask, transfers: Choice::Ask, uploads: Choice::Deny });
        let open = Policy { downloads: Choice::Allow, transfers: Choice::Deny, uploads: Choice::Allow };
        policies.set(open).unwrap();
        assert_eq!(Policies::load(&dir).unwrap().policy, open);
        assert!(!Policy { uploads: Choice::Ask, ..open }.valid(), "uploads are allowed or denied");
    }

    #[tokio::test]
    async fn an_ask_waits_for_the_administrator_and_a_denial_asks_nobody() {
        let session = Session::test("abc", Owner::Admin, crate::app::Features::default());
        session.files.lock().unwrap().at(Some(1));
        let denied = permitted(&session, 1, Choice::Deny, Kind::Download, "a.png", Some(3)).await;
        assert!(denied.is_err() && session.files.lock().unwrap().asks.is_empty());
        let asking = permitted(&session, 1, Choice::Ask, Kind::Transfer, "a.png", Some(3));
        let answering = async {
            let id = loop {
                if let Some(id) = asks(&session)["asks"][0]["id"].as_u64() {
                    break id as u32;
                }
                tokio::task::yield_now().await;
            };
            assert_eq!(asks(&session)["asks"][0]["kind"], "transfer");
            assert!(answer(&session, id, true));
        };
        let (allowed, ()) = tokio::join!(asking, answering);
        assert_eq!(allowed, Ok(()));
        let refusing = async {
            while asks(&session)["asks"][0]["id"].is_null() {
                tokio::task::yield_now().await;
            }
            let id = asks(&session)["asks"][0]["id"].as_u64().unwrap() as u32;
            answer(&session, id, false);
        };
        let (refused, ()) = tokio::join!(permitted(&session, 1, Choice::Ask, Kind::Download, "b", None), refusing);
        assert_eq!(refused, Err("the administrator refused".into()));
        // A lease change refuses what still waits.
        let waiting = permitted(&session, 1, Choice::Ask, Kind::Download, "c", None);
        let moving = async {
            while asks(&session)["asks"][0]["id"].is_null() {
                tokio::task::yield_now().await;
            }
            session.files.lock().unwrap().at(None);
        };
        let (moved, ()) = tokio::join!(waiting, moving);
        assert!(moved.unwrap_err().contains("control moved"));
    }

    #[test]
    fn a_sent_file_is_named_not_pathed() {
        assert!(plain_name("avatar.png") && plain_name("é.jpg"));
        for bad in ["", ".", "..", "a/b", "a\\b", "a\nb", &"x".repeat(MAX_FILE_NAME + 1)] {
            assert!(!plain_name(bad), "{bad:?}");
        }
    }
}
