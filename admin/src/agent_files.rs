//! The agent's files, while it holds control: what the browser downloads, files the agent sends into the browser, and
//! the page's file chooser.
//!
//! Each running session outside the demo keeps one link to its display (`Link::AgentFiles`). While no person holds
//! control, the display sends it what it would send the holder: download offers, the page's choosers, the answers to
//! uploads; the link sends only the file messages a holder would, and the display refuses any other input on it. A
//! lease change ends everything of the previous lease, here as on the display: a file the page was already given it
//! may still read, until a newer upload needs its room (see the display's `uploads`).
//!
//! The administrator's permissions (`/api/agent-files`, kept in `agent-files.json`) say what the agent may do: save a
//! download (`downloads`) and send a file into the browser (`transfers`), each `ask` (the default), `allow` or `deny`.
//! A session's own features apply as well: `downloads` for the first; `uploads` for sending, and alone for giving the
//! files it sent to the page's file chooser, as it does for a person. An ask waits up to [`ASK_FOR`] for
//! the administrator's answer on the session's page. The bytes go by single-use URLs: a 256-bit ticket bound to the
//! session, the caller and the agent's lease epoch, which the agent fetches (a download) or puts the file to, so an
//! agent without a token (the local socket) can use them too. A chat's URL is on its own container's loopback, whatever
//! address the admin answers at, and its ticket is good only through its own socket (see [`through_chat`]): the model's
//! provider sees the URL, and it opens nothing from anywhere else.

use std::collections::HashMap;
use std::path::{Path as FilePath, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use display_protocol::frame::Link;
use display_protocol::{FILE_CHUNK, FILE_WINDOW, MAX_CHOSEN, MAX_FILE_NAME, MAX_UPLOAD, Message};
use display_protocol::{UPLOAD_CHUNK, UPLOAD_WINDOW};
use futures_util::StreamExt;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
}

/// The permissions as installations before 2026-11-01 kept them: also `uploads`, which the session's own uploads switch
/// replaced. Read and dropped, so an update keeps the other two; the next save writes them without it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    downloads: Choice,
    transfers: Choice,
    #[serde(default, rename = "uploads")]
    _uploads: Option<serde::de::IgnoredAny>,
}

impl From<Stored> for Policy {
    fn from(stored: Stored) -> Policy {
        let Stored { downloads, transfers, .. } = stored;
        Policy { downloads, transfers }
    }
}

impl Default for Policy {
    fn default() -> Policy {
        Policy { downloads: Choice::Ask, transfers: Choice::Ask }
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
            Some(bytes) => crate::private::json::<Stored>(&file, &bytes)?.into(),
        };
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
    /// The link ended: opened once ([`spawn`]), it no longer brings a chooser, nor takes one's answer, in this session.
    ended: bool,
    /// The agent's lease epoch they belong to, while it holds control.
    epoch: Option<u32>,
    offers: Vec<Offer>,
    /// The page's open chooser: its request, and whether it takes several files.
    chooser: Option<(u32, bool)>,
    /// The files the agent sent, as the display keeps them: it says when it forgets one (an `UploadDone` that it is
    /// not stored), as it does for a newer upload's room, so how many may be sent has no bound of its own here.
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
        let (next_upload, next_ask, ended) = (self.next_upload, self.next_ask, self.ended);
        *self = Files { link, control, ended, epoch, next_upload, next_ask, ..Files::default() };
    }

    /// Sends a control message; false when the link is gone or its queue full.
    fn send(&self, message: Message) -> bool {
        self.control.as_ref().is_some_and(|control| control.try_send(Link::Relay(message)).is_ok())
    }

    /// Whether the link ended, its end said ([`Files::ended`]) or not yet (its queue closed).
    fn gone(&self) -> bool {
        self.ended || self.control.as_ref().is_some_and(mpsc::Sender::is_closed)
    }
}

/// The agent's lease epoch now, while it holds control.
fn agent_epoch(session: &Session) -> Option<u32> {
    *session.state.lock().unwrap().files_lease.borrow()
}

/// What the file tools say once the files link ended: no file or chooser reaches the agent or the page again in this
/// session.
const LINK_GONE: &str = "this session's link for files is gone: its file tools no longer work in it";

impl Files {
    /// Why the agent's files are not at `epoch`: control moved, or the link ended.
    fn moved(&self) -> String {
        if self.gone() { LINK_GONE } else { "control moved" }.into()
    }
}

/// Opens the session's files link, once, and keeps it until the session ends. A link that fails (or cannot open) is not
/// opened again: for as long as the session runs, the agent's file tools then answer that it is gone ([`LINK_GONE`]).
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
        (files.link, files.control, files.ended) = (None, None, true);
        files.at(None);
    });
}

async fn link(app: &App, session: &Session) -> Result<(), String> {
    let path = app.config.streams_dir.join(&session.id).join("display.sock");
    let mut display = crate::display_link::DisplayLink::open(&path).await?;
    display.send(&Link::AgentFiles).await?;
    let (tx, mut rx) = mpsc::channel(LINK_QUEUE);
    let (control, mut controls) = mpsc::channel(LINK_QUEUE);
    {
        let mut files = session.files.lock().unwrap();
        (files.link, files.control) = (Some(tx), Some(control));
    }
    let mut lease = session.state.lock().unwrap().files_lease.subscribe();
    lease.mark_changed();
    let mut ended = session.ended.subscribe();
    // The page this link last told the timeline of.
    let mut told: Option<Value> = None;
    loop {
        tokio::select! {
            biased;
            _ = async { drop(ended.wait_for(|ended| *ended).await) } => return Ok(()),
            Some(link) = controls.recv() => display.send(&link).await?,
            changed = lease.changed() => {
                changed.map_err(|_| "the session went")?;
                let epoch = *lease.borrow_and_update();
                session.files.lock().unwrap().at(epoch);
                // A grant only while the agent holds control: never one that could race a holder's of the same epoch.
                if let Some(lease_epoch) = epoch {
                    display.send(&Link::Grant { lease_epoch, holder: false }).await?;
                }
            }
            Some(link) = rx.recv() => display.send(&link).await?,
            read = display.read() => {
                for link in read? {
                    match link {
                        Link::Relay(message) => heard(session, message),
                        // While a person holds control the title may show what they are typing: kept without it.
                        // The display tells the title again as control changes; one the same as the last entry kept (cut
                        // to its bound) is not kept again. The display may skip changes when it is busy, and a title may
                        // come again after a hidden one: what is kept follows the window's title, not its pages.
                        Link::PageTitle { title } => {
                            let page = if session.state.lock().unwrap().holder.is_some() {
                                json!({"hidden": true})
                            } else {
                                json!({"title": crate::timeline::cut(&title, crate::timeline::MAX_TEXT)})
                            };
                            if told.as_ref() != Some(&page) {
                                session.agent.timeline.push("page", page.clone());
                                told = Some(page);
                            }
                        }
                        // A new tab closed as it opened: the agent hears why at its next call, and the timeline says so.
                        Link::TabRefused { limit } => {
                            session.state.lock().unwrap().tab_refused = Some(limit);
                            session.agent.timeline.push("tab", json!({"refused": limit}));
                        }
                        // A tab's page crashed: told the same way, with its cause.
                        Link::TabCrashed { memory } => {
                            let mut state = session.state.lock().unwrap();
                            state.tab_crashed = Some(memory || state.tab_crashed == Some(true));
                            drop(state);
                            session.agent.timeline.push("tab", json!({"crashed": if memory { "memory" } else { "other" }}));
                        }
                        _ => {}
                    }
                }
                // What else that read held, which was no frame, ends the link now: nothing more is sent on it.
                display.broken()?;
            }
        }
    }
}

/// What the display sent the agent's link.
fn heard(session: &Session, message: Message) {
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
            session
                .agent
                .timeline
                .push("file", json!({"file": "offered", "name": crate::timeline::kept(&name), "size": size}));
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
            session.agent.timeline.push("file", json!({"file": "chooser", "multiple": multiple}));
            if session.features.uploads {
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
                if let Some(sent) = files.sent.iter().find(|s| s.upload == upload) {
                    let stored =
                        json!({"file": "stored", "name": crate::timeline::kept(&sent.name), "size": sent.size});
                    session.agent.timeline.push("file", stored);
                }
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
            "description": "The files of this session while you hold control: `downloads`, what the browser downloads or downloaded \
                for you, which may be offered while it is still being written (save one with save_download, and start \
                its GET then: it comes as it is written, and one left waiting can fill the browser's room for \
                downloads); `files`, those you sent into the browser (send_file); `chooser`, the \
                file chooser the page opened, if one is open (answer it with choose_files); and `permissions`, what the \
                administrator lets you do (ask: they are asked first; deny: refused), within what the session allows: \
                save_download needs its downloads on, send_file and choose_files its uploads, and are refused otherwise \
                whatever the permission says. After clicking a page's upload button, \
                call this to see its chooser.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": read_only,
        }),
        json!({
            "name": "save_download",
            "description": "Gives you a download the browser made, by its id from list_files: a URL to GET once, within 5 \
                minutes, for its exact bytes (for example `curl -fo <name> '<url>'`). One GET of a download at a time: \
                another while one runs is refused, and spends its URL. A session keeps 8 unused URLs of save_download \
                and send_file together, a ninth dropping the oldest: ask for each as you use it. When the permission is \
                `ask`, this waits up to 2 minutes for the administrator to allow it on the session's page.",
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
                to PUT the file's exact bytes to once, within 5 minutes, with a `Content-Length` of the size given here \
                (for example `curl -fT <file> '<url>'`; a body sent in chunks is refused and the URL is spent), named \
                and sized as given here (a plain name of at most 255 bytes of UTF-8: not `.` or `..`, no `/`, `\\` or \
                control characters). The page sees it trimmed of the whitespace around it, then of the dots it starts \
                with; `download` when nothing is left. The PUT's answer gives the file's id for choose_files. The browser keeps the files \
                within its room: as newer ones need it, older ones go, those a page was already given first, and \
                list_files shows the files you can still choose. A session keeps 8 unused URLs of send_file and \
                save_download together, a ninth dropping the oldest: ask for each as you use it. When the permission is `ask`, this waits up to 2 \
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
                (their ids); none cancels it. A chooser that takes one file takes at most one. Its answer, `chosen`, is \
                how many were passed on: the page may have closed its chooser meanwhile, so see that the page shows \
                the file before relying on it.",
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
    // Once the link ended, nothing a file tool does can reach the page, whoever holds control: said at once, by every
    // one, before any wait that could not end it.
    if session.files.lock().unwrap().gone() {
        return failed(LINK_GONE);
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
                (Some(request), Some(files)) => choose(session, epoch, request, files),
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
        Ok(value) => result(names_redacted(session, value).await),
        Err(why) => failed(why),
    }
}

/// `value` with what a person typed out of its file names, which a page chose. The rest is easi.run's own, its URLs and
/// its fields' names: a word the person typed, taken out of those, would break them for the agent. With the set full,
/// what comes back is what stands for anything an agent reads then.
async fn names_redacted(session: &Session, mut value: Value) -> Value {
    let mut names = Vec::new();
    each_name(&mut value, &mut |name| names.push(name.clone()));
    // Holding no share of the answers' memory, it is never refused for it.
    let (clean, _) = crate::service::redacted(session, Value::Array(names), None).await.expect("no share to grow");
    let Value::Array(clean) = clean else { return clean };
    let mut clean = clean.into_iter();
    each_name(&mut value, &mut |name| {
        if let Some(redacted) = clean.next() {
            *name = redacted;
        }
    });
    value
}

/// Each file name in one of these tools' answers: its own, and those it lists.
fn each_name(value: &mut Value, each: &mut impl FnMut(&mut Value)) {
    if let Some(name) = value.get_mut("name") {
        each(name);
    }
    for list in ["downloads", "files"] {
        for item in value.get_mut(list).and_then(Value::as_array_mut).into_iter().flatten() {
            if let Some(name) = item.get_mut("name") {
                each(name);
            }
        }
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
async fn asked(
    session: &Session,
    epoch: u32,
    kind: Kind,
    name: &str,
    size: Option<u64>,
    told: impl FnOnce(),
) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    let id = {
        let mut files = session.files.lock().unwrap();
        if files.epoch != Some(epoch) {
            return Err(files.moved());
        }
        if files.asks.len() >= MAX_ASKS {
            return Err("too many questions wait for the administrator already".into());
        }
        files.next_ask = files.next_ask.wrapping_add(1).max(1);
        let id = files.next_ask;
        files.asks.push(Ask { id, kind, name: name.to_owned(), size, answer: tx });
        id
    };
    session.agent.timeline.touch();
    // Gone with the wait, however it ends: a caller that leaves takes its question with it, and the timeline hears how
    // it ended (cancelled, unless an answer or the bound came first).
    struct Asking<'a> {
        session: &'a Session,
        id: u32,
        event: Value,
        ended: &'static str,
    }
    impl Drop for Asking<'_> {
        fn drop(&mut self) {
            self.session.files.lock().unwrap().asks.retain(|a| a.id != self.id);
            self.event["state"] = json!(self.ended);
            self.session.agent.timeline.push("ask", self.event.take());
        }
    }
    let event = json!({"ask": id, "for": kind, "name": crate::timeline::kept(name), "size": size, "state": "waiting"});
    session.agent.timeline.push("ask", event.clone());
    told();
    let mut asking = Asking { session, id, event, ended: "cancelled" };
    let answer = tokio::time::timeout(ASK_FOR, rx).await;
    asking.ended = match answer {
        Ok(Ok(true)) => "allowed",
        Ok(Ok(false)) => "refused",
        Ok(Err(_)) => "cancelled",
        Err(_) => "expired",
    };
    drop(asking);
    match answer {
        Ok(Ok(true)) => Ok(()),
        Ok(Ok(false)) => Err("the administrator refused".into()),
        Ok(Err(_)) if session.files.lock().unwrap().gone() => Err(LINK_GONE.into()),
        Ok(Err(_)) => Err("control moved before the administrator answered".into()),
        Err(_) => Err(format!(
            "nobody answered within {} s: ask the person to open the session's watch_url to answer, then try again",
            ASK_FOR.as_secs()
        )),
    }
}

/// Tells the webhook an agent's file waits for the administrator (see `notify`): its kind, never its name.
fn approval_needed(app: &App, session: &Session, kind: &str) {
    let watch_url = crate::service::watch_url(app, &session.id);
    let label = crate::service::label(session);
    let about = crate::notify::About { id: &session.id, label: &label, watch_url: &watch_url };
    app.notify.send(crate::notify::Event::ApprovalNeeded { kind }, Some(about));
}

/// The permission `choice`, asked when it says so (`told` once the question waits).
async fn permitted(
    session: &Session,
    epoch: u32,
    choice: Choice,
    kind: Kind,
    name: &str,
    size: Option<u64>,
    told: impl FnOnce(),
) -> Result<(), String> {
    match choice {
        Choice::Allow => Ok(()),
        Choice::Deny => Err(match kind {
            Kind::Download => "the administrator does not let agents save downloads".into(),
            Kind::Transfer => "the administrator does not let agents send files into the browser".into(),
        }),
        Choice::Ask => asked(session, epoch, kind, name, size, told).await,
    }
}

/// Where a chat reaches its files: the forwarder `runner/start` keeps on its container's loopback, to its socket here.
const CHAT_ORIGIN: &str = "http://127.0.0.1:3129";

/// A new single-use URL for `what`.
fn ticket(app: &App, who: Owner, session: &Session, epoch: u32, what: Use) -> Result<String, String> {
    let mut key = [0u8; 32];
    rand::rng().fill_bytes(&mut key);
    let mut files = session.files.lock().unwrap();
    if files.epoch != Some(epoch) {
        return Err(files.moved());
    }
    files.tickets.retain(|t| t.until > Instant::now());
    if files.tickets.len() >= MAX_TICKETS {
        files.tickets.remove(0);
    }
    files.tickets.push(Ticket { key, who, epoch, until: Instant::now() + TICKET_FOR, what });
    let key = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key);
    let origin = if matches!(who, Owner::Chat(..)) { CHAT_ORIGIN } else { app.config.origin.as_str() };
    Ok(format!("{origin}/api/agent/sessions/{}/files/{key}", session.id))
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
    permitted(session, epoch, choice, Kind::Download, &name, size, || approval_needed(app, session, "download"))
        .await?;
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
    let choice = app.agent_files.lock().unwrap().policy.transfers;
    let told = || approval_needed(app, session, "transfer");
    permitted(session, epoch, choice, Kind::Transfer, &name, Some(size), told).await?;
    let url = ticket(app, who, session, epoch, Use::Send { name, size })?;
    Ok(json!({"url": url, "expires_in_s": TICKET_FOR.as_secs()}))
}

fn choose(session: &Session, epoch: u32, request: u32, chosen: Vec<u32>) -> Result<Value, String> {
    if !session.features.uploads {
        return Err("this session's uploads are off".into());
    }
    let mut files = session.files.lock().unwrap();
    let Some((open, multiple)) = files.chooser.filter(|_| files.epoch == Some(epoch)) else {
        return Err("no file chooser is open: click the page's upload button, then list_files".into());
    };
    if open != request {
        return Err("that chooser closed; list_files shows the one open now".into());
    }
    let most = if multiple { MAX_CHOSEN } else { 1 };
    if chosen.len() > most {
        return Err(format!("this chooser takes at most {most} file{}", if most == 1 { "" } else { "s" }));
    }
    if !chosen.iter().all(|id| files.sent.iter().any(|s| s.upload == *id && s.stored)) {
        return Err("no such file: list_files lists the files you sent".into());
    }
    let count = chosen.len();
    if !files.send(Message::FileChoose { lease_epoch: epoch, request, uploads: chosen }) {
        return Err(if files.gone() { LINK_GONE } else { "the session is busy; try again" }.into());
    }
    files.chooser = None;
    session.agent.timeline.push("file", json!({"file": "chosen", "count": count}));
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
    let answered = files.asks.remove(index).answer.send(allow).is_ok();
    drop(files);
    session.agent.timeline.touch();
    answered
}

fn refused(status: StatusCode, code: Code, why: &str) -> Response {
    error(status, code, why)
}

/// Whether a ticket of `who` is good on the route `through`: a chat's only through its own socket, any other only on the
/// admin's own address. One asked for on the wrong route stays good on the right one.
fn good_through(who: Owner, through: Option<Owner>) -> bool {
    if matches!(who, Owner::Chat(..)) { through == Some(who) } else { through.is_none() }
}

/// The ticket `key` of session `id`, taken out (single use), if it is still good: on this route, unexpired, of the agent's
/// lease now, and of a caller that may still act on the session.
fn take(app: &App, id: &str, key: &str, through: Option<Owner>) -> Result<(Arc<Session>, Ticket), Box<Response>> {
    let none = || Box::new(refused(StatusCode::NOT_FOUND, Code::NoDownload, "no such file URL"));
    let key: [u8; 32] = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(key)
        .ok()
        .and_then(|k| k.try_into().ok())
        .ok_or_else(none)?;
    let session = app.session(id).ok_or_else(none)?;
    let ticket = {
        let mut files = session.files.lock().unwrap();
        let good = |t: &Ticket| t.key == key && t.until > Instant::now() && good_through(t.who, through);
        let index = files.tickets.iter().position(good).ok_or_else(none)?;
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
pub async fn fetch(State(app): State<Shared>, Path(url): Path<(String, String)>) -> Response {
    fetch_through(app, url, None).await
}

async fn fetch_through(app: Shared, (id, key): (String, String), through: Option<Owner>) -> Response {
    let (session, ticket) = match take(&app, &id, &key, through) {
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
        // One response per download: another URL of it, while this one runs, is refused (and spent).
        if files.receiving.contains_key(&file) {
            return refused(StatusCode::CONFLICT, Code::DownloadsBusy, "this download is being received already");
        }
        files.receiving.insert(file, tx);
        link
    };
    let epoch = ticket.epoch;
    // Its guard first: a response dropped while the first credit waits for room still lets go of the download.
    let receiving = Receiving { session, file, name: name.clone(), epoch, complete: false };
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
                    let saved = json!({"file": "saved", "name": crate::timeline::kept(&receiving.name), "size": taken});
                    receiving.session.agent.timeline.push("file", saved);
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
    name: String,
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
    Path(url): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    put_through(app, url, headers, body, None).await
}

/// What a chat's file socket answers, for `who` alone: its files' URLs and nothing else of the admin.
pub fn through_chat(app: Shared, who: Owner) -> axum::Router {
    let fetching = move |State(app): State<Shared>, Path(url): Path<(String, String)>| async move {
        fetch_through(app, url, Some(who)).await
    };
    let putting = move |State(app): State<Shared>,
                        Path(url): Path<(String, String)>,
                        headers: HeaderMap,
                        body: Body| async move { put_through(app, url, headers, body, Some(who)).await };
    axum::Router::new()
        .route(
            "/api/agent/sessions/{id}/files/{ticket}",
            axum::routing::get(fetching).head(crate::api::no_head).put(putting),
        )
        .with_state(app)
}

async fn put_through(
    app: Shared,
    (id, key): (String, String),
    headers: HeaderMap,
    body: Body,
    through: Option<Owner>,
) -> Response {
    let (session, ticket) = match take(&app, &id, &key, through) {
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
        let Some(link) = files.link.clone().filter(|_| files.epoch == Some(epoch)) else {
            return refused(StatusCode::GONE, Code::ControlMoved, "control moved");
        };
        files.next_upload = files.next_upload.wrapping_add(1).max(1);
        let upload = files.next_upload;
        files.sent.push(Sent { upload, name: name.clone(), size, stored: false });
        files.sending.insert(upload, progress);
        (upload, link)
    };
    // However the request ends before the upload's start is queued on the display's link (its client gone mid-way
    // included), it is listed no more; once it is queued, the link delivers it whatever becomes of the request, and
    // the display's answer says whether the browser stored it, as for any upload.
    let reached = std::sync::atomic::AtomicBool::new(false);
    let _unsent = Unsent { session: &session, upload, reached: &reached };
    let sent = async {
        let start = Message::UploadStart { lease_epoch: epoch, upload, size, name: name.clone() };
        link.send(Link::Relay(start)).await.map_err(|_| "the session's link closed")?;
        reached.store(true, std::sync::atomic::Ordering::Relaxed);
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
        Err(why) => refused(StatusCode::CONFLICT, Code::BadRequest, why),
    }
}

/// A file the agent sends into the browser, while its request runs: if the request ends before the upload's start is
/// queued on the display's link (`reached`), nothing will ever say what became of it, and it is forgotten. Once it is
/// queued, the link delivers it, the display answers whether it was stored (an upload it stops hearing from fails
/// within its own time), and that answer keeps or forgets it ([`heard`]).
struct Unsent<'a> {
    session: &'a Session,
    upload: u32,
    reached: &'a std::sync::atomic::AtomicBool,
}

impl Drop for Unsent<'_> {
    fn drop(&mut self) {
        if !self.reached.load(std::sync::atomic::Ordering::Relaxed) {
            let mut files = self.session.files.lock().unwrap();
            files.sent.retain(|s| s.upload != self.upload);
            files.sending.remove(&self.upload);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chats_ticket_is_good_through_its_own_socket_alone() {
        let chat = Owner::Chat(*b"000000000001", 7);
        assert!(good_through(chat, Some(chat)));
        assert!(!good_through(chat, None), "on the admin's own address");
        assert!(!good_through(chat, Some(Owner::Chat(*b"000000000001", 8))), "through a later chat of the session");
        assert!(!good_through(chat, Some(Owner::Chat(*b"000000000002", 7))), "through another session's chat");
        assert!(good_through(Owner::Token(1), None));
        assert!(!good_through(Owner::Token(1), Some(chat)), "another's, through a chat's socket");
    }

    #[test]
    fn a_file_whose_request_ended_before_the_display_had_it_is_listed_no_more() {
        let session = Session::test("abc", Owner::Admin, crate::app::Features::default());
        for upload in [1, 2] {
            let mut files = session.files.lock().unwrap();
            files.sent.push(Sent { upload, name: "a.png".into(), size: 3, stored: false });
            files.sending.insert(upload, watch::channel(Progress::default()).0);
        }
        let (before, after) = (std::sync::atomic::AtomicBool::new(false), std::sync::atomic::AtomicBool::new(true));
        drop(Unsent { session: &session, upload: 1, reached: &before });
        drop(Unsent { session: &session, upload: 2, reached: &after });
        assert_eq!(session.files.lock().unwrap().sent.iter().map(|s| s.upload).collect::<Vec<_>>(), [2]);
        // The display has the other: its answer, even after the request went, says what became of it.
        session.files.lock().unwrap().at(Some(1));
        let sent = Sent { upload: 2, name: "a.png".into(), size: 3, stored: false };
        session.files.lock().unwrap().sent.push(sent);
        heard(&session, Message::UploadDone { lease_epoch: 1, upload: 2, stored: true });
        assert!(session.files.lock().unwrap().sent.iter().any(|s| s.upload == 2 && s.stored));
    }

    #[test]
    fn a_file_the_display_forgot_is_no_longer_listed() {
        let session = Session::test("abc", Owner::Admin, crate::app::Features::default());
        let sent = |upload: u32| Sent { upload, name: "a.png".into(), size: 3, stored: true };
        session.files.lock().unwrap().at(Some(1));
        session.files.lock().unwrap().sent.extend((1..=3).map(sent));
        // The display forgot a stored one for a newer upload's room.
        heard(&session, Message::UploadDone { lease_epoch: 1, upload: 2, stored: false });
        let listed: Vec<u32> = session.files.lock().unwrap().sent.iter().map(|s| s.upload).collect();
        assert_eq!(listed, [1, 3]);
    }

    #[test]
    fn the_permissions_start_at_the_safest_and_are_kept() {
        let dir = crate::private::test_dir("agent-files");
        let mut policies = Policies::load(&dir).unwrap();
        assert_eq!(policies.policy, Policy { downloads: Choice::Ask, transfers: Choice::Ask });
        let open = Policy { downloads: Choice::Allow, transfers: Choice::Deny };
        policies.set(open).unwrap();
        assert_eq!(Policies::load(&dir).unwrap().policy, open);
        // As an earlier installation kept them, with `uploads`: the others stay.
        let earlier = br#"{"downloads": "allow", "transfers": "deny", "uploads": "allow"}"#;
        crate::private::replace(&dir.join("agent-files.json"), earlier).unwrap();
        assert_eq!(Policies::load(&dir).unwrap().policy, open);
    }

    #[tokio::test]
    async fn an_ask_waits_for_the_administrator_and_a_denial_asks_nobody() {
        let session = Session::test("abc", Owner::Admin, crate::app::Features::default());
        session.files.lock().unwrap().at(Some(1));
        let denied = permitted(&session, 1, Choice::Deny, Kind::Download, "a.png", Some(3), || ()).await;
        assert!(denied.is_err() && session.files.lock().unwrap().asks.is_empty());
        let asking = permitted(&session, 1, Choice::Ask, Kind::Transfer, "a.png", Some(3), || ());
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
        let (refused, ()) =
            tokio::join!(permitted(&session, 1, Choice::Ask, Kind::Download, "b", None, || ()), refusing);
        assert_eq!(refused, Err("the administrator refused".into()));
        // A lease change refuses what still waits.
        let waiting = permitted(&session, 1, Choice::Ask, Kind::Download, "c", None, || ());
        let moving = async {
            while asks(&session)["asks"][0]["id"].is_null() {
                tokio::task::yield_now().await;
            }
            session.files.lock().unwrap().at(None);
        };
        let (moved, ()) = tokio::join!(waiting, moving);
        assert!(moved.unwrap_err().contains("control moved"));
    }

    #[tokio::test(start_paused = true)]
    async fn an_ask_nobody_answers_expires_says_where_to_answer_and_the_timeline_says_so() {
        let session = Session::test("abc", Owner::Admin, crate::app::Features::default());
        session.files.lock().unwrap().at(Some(1));
        let told = std::cell::Cell::new(false);
        let expired = permitted(&session, 1, Choice::Ask, Kind::Download, "d", None, || told.set(true)).await;
        assert!(told.get(), "the webhook is told once the question waits");
        assert!(expired.unwrap_err().contains("watch_url"));
        assert!(session.files.lock().unwrap().asks.is_empty());
        let events = session.agent.timeline.page(0)["events"].clone();
        assert_eq!(events.as_array().unwrap().last().unwrap()["state"], "expired");
    }

    #[test]
    fn a_sent_file_is_named_not_pathed() {
        assert!(plain_name("avatar.png") && plain_name("é.jpg"));
        for bad in ["", ".", "..", "a/b", "a\\b", "a\nb", &"x".repeat(MAX_FILE_NAME + 1)] {
            assert!(!plain_name(bad), "{bad:?}");
        }
    }
}
