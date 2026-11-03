//! Chats: a person's own coding agent beside one of their sessions, on a terminal over the screen. A chat runs one
//! program, Claude Code or Codex, as its vendor's own installer puts it, unmodified, in the home of an account the
//! administrator named; signing in happens there, by the program's own flow, and what it keeps of that stays in that
//! home, which the admin never reads. Several accounts of each program may exist, and a person picks one for a session.
//! Never in the demo.
//!
//! What the program gets of the session is the MCP socket the admin serves for that chat alone (see `local`): the
//! principal [`Owner::Chat`] reaches that one session, needing no token and no saved session's allow-list, and starts,
//! stops and lists nothing else. Its calls obey what every agent's do while a person holds control. Unlike any other
//! principal's, its results keep their screenshots after a takeover: the person opened this agent on their own screen.
//!
//! The controller runs the program in a container of its own, with no network but its own proxy (see docs/topology.md),
//! on a terminal that outlives every viewer. A session has one chat at most, an account one chat at a time, and stopping
//! the session ends its chat. A restart of the admin ends them, as it ends the sessions.
//!
//! `chat-accounts.json` in the state directory lists the accounts (mode 0600, replaced atomically): names and programs.
//! Removing one removes its home.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::api::{Administrator, Shared, coded, error, no_session};
use crate::app::{App, Owner, Session};
use crate::auth::{LoginKey, unix_now};
use crate::coded::{Code, Coded};

const FILE: &str = "chat-accounts.json";
/// Accounts at once.
const MAX_ACCOUNTS: usize = 32;
/// The MCP socket's name in the chat's directory; the runner image's `runner-mcp` connects to it.
const SOCKET: &str = "mcp.sock";
/// How often a terminal looks at whether its login is still live.
const LOGIN_EVERY: Duration = Duration::from_secs(5);
/// The largest message a terminal takes from a viewer: a paste.
const MAX_INPUT: usize = 1024 * 1024;
/// How many times a terminal whose stream ended looks for its program's container to have stopped, half a second apart.
const PROGRAM_END_LOOKS: u32 = 10;
/// How often a chat's program is looked at, for one that ended with no terminal open.
const REAP_EVERY: Duration = Duration::from_secs(30);
/// How long a write to a viewer or to the terminal may wait: past it, that viewer is gone.
const WRITE_WITHIN: Duration = crate::deadline::WRITE;
/// How a terminal ends, as its socket's close code says: the program ended, or another terminal took its place.
const CLOSE_ENDED: u16 = 4000;
const CLOSE_REPLACED: u16 = 4001;

/// The program a chat runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Product {
    Claude,
    Codex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub product: Product,
    pub created: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    accounts: Vec<Account>,
}

pub struct Accounts {
    file: PathBuf,
    list: Vec<Account>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    BadName,
    TooMany,
    Unsaved(String),
}

impl Accounts {
    pub fn empty(state_dir: &Path) -> Accounts {
        Accounts { file: state_dir.join(FILE), list: Vec::new() }
    }

    /// The accounts on file; a file that cannot be read whole is refused: starting without it would orphan their homes.
    pub fn load(state_dir: &Path) -> Result<Accounts, String> {
        let mut accounts = Accounts::empty(state_dir);
        if let Some(bytes) = crate::private::read(&accounts.file)? {
            let stored: Stored = crate::private::json(&accounts.file, &bytes)?;
            accounts.list = stored.accounts;
        }
        Ok(accounts)
    }

    fn save(&self) -> Result<(), String> {
        let stored = Stored { accounts: self.list.clone() };
        let bytes = serde_json::to_vec_pretty(&stored).map_err(|e| e.to_string())?;
        crate::private::installed(crate::private::replace(&self.file, &bytes))
            .map(|_| ())
            .map_err(|why| why.to_string())
    }

    pub fn list(&self) -> &[Account] {
        &self.list
    }

    pub fn get(&self, id: &str) -> Option<&Account> {
        self.list.iter().find(|a| a.id == id)
    }

    pub fn create(&mut self, name: &str, product: Product) -> Result<Account, Refused> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > crate::tokens::MAX_NAME || name.chars().any(char::is_control) {
            return Err(Refused::BadName);
        }
        if self.list.len() >= MAX_ACCOUNTS {
            return Err(Refused::TooMany);
        }
        let id = loop {
            let id = format!("{:016x}", rand::random::<u64>());
            if self.get(&id).is_none() {
                break id;
            }
        };
        let account = Account { id, name: name.to_owned(), product, created: unix_now() };
        self.list.push(account.clone());
        if let Err(why) = self.save() {
            self.list.pop();
            return Err(Refused::Unsaved(why));
        }
        Ok(account)
    }

    /// Forgets an account; false when it could not be written, and then it stays.
    fn remove(&mut self, id: &str) -> Result<(), String> {
        let before = self.list.clone();
        self.list.retain(|a| a.id != id);
        self.save().inspect_err(|_| self.list = before)
    }
}

/// A session's chat, while it runs.
pub struct Chat {
    pub account: Account,
    /// This chat among the session's: calls an earlier one left waiting never act as this one.
    generation: u64,
    /// The latest terminal opened on it: an earlier one, replaced, says so as it ends.
    terminal: u64,
    /// Serves its MCP socket.
    serving: Option<tokio::task::AbortHandle>,
}

impl Chat {
    /// As the page reads it.
    pub fn shown(&self) -> Value {
        json!({"account": self.account.id, "name": self.account.name, "product": self.account.product})
    }
}

impl Drop for Chat {
    fn drop(&mut self) {
        if let Some(serving) = &self.serving {
            serving.abort();
        }
    }
}

/// The principal of `session`'s chat.
pub fn owner(session: &Session) -> Option<Owner> {
    let generation = session.chat.lock().unwrap().as_ref()?.generation;
    Some(Owner::Chat(session.id.as_bytes().try_into().ok()?, generation))
}

/// Where the admin keeps `session`'s chat's socket: the directory the runner mounts read-only.
fn directory(app: &App, session: &str) -> PathBuf {
    app.config.streams_dir.join(format!("chat-{session}"))
}

/// Makes the chat's directory and binds its socket, for the runner's uid to connect to and no one else.
fn bind(dir: &Path) -> Result<tokio::net::UnixListener, String> {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o750)).map_err(|e| e.to_string())?;
    let path = dir.join(SOCKET);
    let listener = tokio::net::UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660)).map_err(|e| e.to_string())?;
    Ok(listener)
}

fn busy(why: &str) -> Response {
    error(StatusCode::CONFLICT, Code::ChatBusy, why)
}

fn described(app: &Shared, account: &Account) -> Value {
    let used = app.sessions.lock().unwrap().values().find_map(|s| {
        s.chat.lock().unwrap().as_ref().is_some_and(|c| c.account.id == account.id).then(|| s.id.clone())
    });
    json!({"id": account.id, "name": account.name, "product": account.product, "session": used})
}

pub async fn list(State(app): State<Shared>, _: Administrator) -> Response {
    let accounts: Vec<Account> = app.chat_accounts.lock().unwrap().list().to_vec();
    Json(json!({"accounts": accounts.iter().map(|a| described(&app, a)).collect::<Vec<_>>()})).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewAccount {
    name: String,
    product: Product,
}

pub async fn create(State(app): State<Shared>, _: Administrator, Json(body): Json<NewAccount>) -> Response {
    let created = app.chat_accounts.lock().unwrap().create(&body.name, body.product);
    match created {
        Ok(account) => {
            tracing::info!(account = account.id, "chat account created");
            Json(described(&app, &account)).into_response()
        }
        Err(Refused::BadName) => {
            let why = format!("name it in 1 to {} characters", crate::tokens::MAX_NAME);
            coded(StatusCode::BAD_REQUEST, Coded::new(Code::TokenName, why).with("max", crate::tokens::MAX_NAME))
        }
        Err(Refused::TooMany) => {
            error(StatusCode::CONFLICT, Code::TooManyChatAccounts, format!("at most {MAX_ACCOUNTS} accounts"))
        }
        Err(Refused::Unsaved(why)) => error(StatusCode::INTERNAL_SERVER_ERROR, Code::ServerUnsaved, why),
    }
}

/// Removes an account and its home: the program, its login and its conversations. Not while a chat uses it.
pub async fn remove(State(app): State<Shared>, _: Administrator, UrlPath(id): UrlPath<String>) -> Response {
    let Some(account) = app.chat_accounts.lock().unwrap().get(&id).cloned() else {
        return error(StatusCode::NOT_FOUND, Code::NoChatAccount, "no such account");
    };
    let _changing = app.chat_changes.lock().await;
    if !described(&app, &account)["session"].is_null() {
        return busy("a chat is using this account");
    }
    if let Err(why) = app.controller.account_remove(&id).await {
        return error(StatusCode::BAD_GATEWAY, Code::ChatFailed, why);
    }
    match app.chat_accounts.lock().unwrap().remove(&id) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(why) => error(StatusCode::INTERNAL_SERVER_ERROR, Code::ServerUnsaved, why),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartChat {
    account: String,
}

/// Starts a chat beside the session, with the account picked. A chat whose program ended with no terminal open counts
/// for nothing: found so, it goes first.
pub async fn start(
    State(app): State<Shared>,
    _: Administrator,
    UrlPath(id): UrlPath<String>,
    Json(body): Json<StartChat>,
) -> Response {
    let _changing = app.chat_changes.lock().await;
    let Some(session) = app.session_for(Owner::Admin, &id) else { return no_session() };
    let Some(account) = app.chat_accounts.lock().unwrap().get(&body.account).cloned() else {
        return error(StatusCode::NOT_FOUND, Code::NoChatAccount, "no such account");
    };
    let others: Vec<Arc<Session>> = app.sessions.lock().unwrap().values().cloned().collect();
    for other in others {
        let used = other.chat.lock().unwrap().as_ref().is_some_and(|c| c.account.id == account.id);
        if used || (other.id == session.id && other.chat.lock().unwrap().is_some()) {
            if runner_running(&app, &other.id).await != Some(false) {
                return busy(if used { "a chat is using this account" } else { "this session has a chat" });
            }
            if let Err(why) = end_chat(&app, &other).await {
                return error(StatusCode::BAD_GATEWAY, Code::ChatFailed, why);
            }
        }
    }
    let product = account.product;
    {
        let mut chat = session.chat.lock().unwrap();
        if chat.is_some() {
            return busy("this session has a chat");
        }
        *chat = Some(Chat { account: account.clone(), generation: rand::random(), terminal: 0, serving: None });
    }
    let dir = directory(&app, &session.id);
    let listener = match bind(&dir) {
        Ok(listener) => listener,
        Err(why) => {
            *session.chat.lock().unwrap() = None;
            return error(StatusCode::INTERNAL_SERVER_ERROR, Code::ChatFailed, why);
        }
    };
    // Gone already when the session stopped meanwhile.
    let Some(who) = owner(&session) else {
        return no_session();
    };
    let generation = session.chat.lock().unwrap().as_ref().map_or(0, |chat| chat.generation);
    if let Owner::Chat(id, generation) = who {
        app.chats_running.lock().unwrap().insert(id, (generation, account.name.clone()));
    }
    let (served, watched) = (app.clone(), app.clone());
    let watching = session.clone();
    let serving = tokio::spawn(async move {
        tokio::select! {
            () = crate::local::serve_chat(listener, served, who) => {}
            () = reap_when_ended(watched, watching, generation) => {}
        }
    });
    if let Some(chat) = session.chat.lock().unwrap().as_mut() {
        chat.serving = Some(serving.abort_handle());
    }
    if let Err(why) = app.controller.chat_start(&session.id, &account.id, product).await {
        // Its answer may be what was lost: the account is free only once nothing of it can be running. Otherwise it stays
        // reserved, for the reaper or a stop to end.
        if app.controller.chat_stop(&session.id).await.is_ok() {
            forget(&app, &session);
        }
        tracing::warn!(session = %session.id, %why, "a chat did not start");
        return error(StatusCode::BAD_GATEWAY, Code::ChatFailed, why);
    }
    tracing::info!(session = %session.id, account = account.id, "chat started");
    Json(json!({"chat": {"account": account.id, "name": account.name, "product": product}})).into_response()
}

pub async fn stop(State(app): State<Shared>, _: Administrator, UrlPath(id): UrlPath<String>) -> Response {
    let Some(session) = app.session_for(Owner::Admin, &id) else { return no_session() };
    match stop_chat(&app, &session, None).await {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(why) => error(StatusCode::BAD_GATEWAY, Code::ChatFailed, why),
    }
}

/// Whether the chat's program is running, if the controller could say.
async fn runner_running(app: &Shared, session: &str) -> Option<bool> {
    let inspected = app.controller.inspect_session(session).await.ok()?;
    let runner = &inspected["containers"]["runner"];
    // No runner recorded is none running; one that could not be inspected says nothing.
    if runner.is_null() { Some(false) } else { runner["running"].as_bool() }
}

/// Ends the chat once its program has ended, with no terminal open to notice: its account is free again.
async fn reap_when_ended(app: Shared, session: Arc<Session>, generation: u64) {
    let mut every = tokio::time::interval(REAP_EVERY);
    every.tick().await;
    loop {
        every.tick().await;
        // A removal that fails is tried again at the next look.
        if runner_running(&app, &session.id).await == Some(false)
            && stop_chat(&app, &session, Some(generation)).await.is_ok()
        {
            return;
        }
    }
}

/// Whether a terminal's stream ended because its program did: the container follows it within a few seconds.
async fn program_ended(app: &Shared, session: &str) -> bool {
    for _ in 0..PROGRAM_END_LOOKS {
        if runner_running(app, session).await == Some(false) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

/// Ends the session's chat, or, given `only`, that chat alone: one that noticed its program end never ends the next.
async fn stop_chat(app: &Shared, session: &Arc<Session>, only: Option<u64>) -> Result<(), String> {
    let _changing = app.chat_changes.lock().await;
    let current = session.chat.lock().unwrap().as_ref().map(|chat| chat.generation);
    if only.is_some() && current != only {
        return Ok(());
    }
    end_chat(app, session).await
}

async fn end_chat(app: &Shared, session: &Arc<Session>) -> Result<(), String> {
    app.controller.chat_stop(&session.id).await?;
    forget(app, session);
    Ok(())
}

/// What the admin holds of a chat whose containers are gone: its record, which stops serving its socket, and the socket.
pub fn forget(app: &App, session: &Session) {
    if let Ok(id) = session.id.as_bytes().try_into() {
        app.chats_running.lock().unwrap().remove::<[u8; 12]>(&id);
    }
    *session.chat.lock().unwrap() = None;
    let _ = std::fs::remove_dir_all(directory(app, &session.id));
}

/// A terminal on the session's chat: the latest one stays, an earlier one is told it was replaced.
pub async fn terminal(
    State(app): State<Shared>,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let login = match app.auth.lock().unwrap().check_upgrade(&headers, &app.config.origin) {
        Ok(key) => key,
        Err(r) => return r.into_response(),
    };
    if app.config.demo.is_some() {
        return crate::api::not_in_the_demo();
    }
    let Some(session) = app.session_for(app.principal(login), &id) else { return no_session() };
    let opened = session.chat.lock().unwrap().as_mut().map(|chat| {
        chat.terminal += 1;
        (chat.generation, chat.terminal)
    });
    let Some(opened) = opened else {
        return error(StatusCode::NOT_FOUND, Code::NoChat, "this session has no chat");
    };
    upgrade.max_message_size(MAX_INPUT).on_upgrade(move |socket| run(app, session, login, opened, socket))
}

/// How a terminal's relay ended.
enum Ended {
    /// The viewer left; the program goes on.
    Viewer,
    /// The exec's stream ended: the program ended, or a later terminal replaced this one.
    Stream,
    /// The session ended, or the login did.
    Closed,
}

/// What a viewer's text message says: the terminal's size.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Size {
    cols: u16,
    rows: u16,
}

async fn run(app: Shared, session: Arc<Session>, login: LoginKey, opened: (u64, u64), mut socket: WebSocket) {
    let (generation, opened) = opened;
    // Its chat may have ended while the upgrade was under way: this terminal never attaches to the next one.
    if session.chat.lock().unwrap().as_ref().is_none_or(|chat| chat.generation != generation) {
        close(&mut socket, CLOSE_REPLACED, "").await;
        return;
    }
    let stream = match app.controller.chat_attach(&session.id).await {
        Ok(stream) => stream,
        Err(why) => {
            tracing::warn!(session = %session.id, %why, "a chat's terminal did not attach");
            if program_ended(&app, &session.id).await {
                let _ = stop_chat(&app, &session, Some(generation)).await;
                close(&mut socket, CLOSE_ENDED, "").await;
            } else {
                close(&mut socket, 1011, "the terminal did not attach").await;
            }
            return;
        }
    };
    let (mut output, mut input) = tokio::io::split(stream);
    let mut ended = session.ended.subscribe();
    let mut login_check = tokio::time::interval(LOGIN_EVERY);
    let mut buffer = vec![0u8; 64 * 1024];
    let how = loop {
        tokio::select! {
            read = output.read(&mut buffer) => match read {
                Ok(n) if n > 0 => {
                    let sent = tokio::time::timeout(WRITE_WITHIN, socket.send(Message::Binary(buffer[..n].to_vec().into())));
                    if !matches!(sent.await, Ok(Ok(()))) {
                        break Ended::Viewer;
                    }
                }
                _ => break Ended::Stream,
            },
            message = socket.recv() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    if !matches!(tokio::time::timeout(WRITE_WITHIN, input.write_all(&bytes)).await, Ok(Ok(()))) {
                        break Ended::Stream;
                    }
                }
                Some(Ok(Message::Text(text))) => {
                    if let Ok(size) = serde_json::from_str::<Size>(&text) {
                        // A size the terminal cannot take changes nothing; the next one may.
                        let _ = app.controller.chat_resize(&session.id, size.cols, size.rows).await;
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => break Ended::Viewer,
            },
            () = async { let _ = ended.wait_for(|ended| *ended).await; } => break Ended::Closed,
            _ = login_check.tick() => {
                if !app.auth.lock().unwrap().is_live(&login) {
                    break Ended::Closed;
                }
            }
        }
    };
    match how {
        Ended::Viewer => {}
        Ended::Closed => close(&mut socket, 1000, "").await,
        Ended::Stream => {
            let replaced = session
                .chat
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|chat| chat.generation != generation || chat.terminal != opened);
            if replaced {
                close(&mut socket, CLOSE_REPLACED, "").await;
            } else if program_ended(&app, &session.id).await {
                let _ = stop_chat(&app, &session, Some(generation)).await;
                close(&mut socket, CLOSE_ENDED, "").await;
            } else {
                close(&mut socket, 1011, "the terminal ended").await;
            }
        }
    }
}

async fn close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = socket.send(Message::Close(Some(CloseFrame { code, reason: reason.into() }))).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_are_kept_with_a_name_and_a_program_and_survive_a_restart() {
        let dir = crate::private::test_dir("chat");
        let mut accounts = Accounts::load(&dir).unwrap();
        let first = accounts.create(" xpto ", Product::Codex).unwrap();
        assert_eq!(first.name, "xpto");
        assert!(first.id.len() == 16 && first.id.bytes().all(|b| b.is_ascii_hexdigit()));
        let second = accounts.create("xpto", Product::Claude).unwrap();
        assert_ne!(first.id, second.id, "the same name twice is two accounts");
        assert_eq!(accounts.create(" ", Product::Codex), Err(Refused::BadName));
        assert_eq!(accounts.create(&"x".repeat(crate::tokens::MAX_NAME + 1), Product::Codex), Err(Refused::BadName));

        accounts.remove(&first.id).unwrap();
        let restarted = Accounts::load(&dir).unwrap();
        assert_eq!(restarted.list().len(), 1);
        assert_eq!(restarted.get(&second.id).unwrap().product, Product::Claude);
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join(FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn no_more_accounts_than_the_most() {
        let dir = crate::private::test_dir("chat");
        let mut accounts = Accounts::load(&dir).unwrap();
        for n in 0..MAX_ACCOUNTS {
            accounts.create(&n.to_string(), Product::Codex).unwrap();
        }
        assert_eq!(accounts.create("one more", Product::Codex), Err(Refused::TooMany));
    }
}
