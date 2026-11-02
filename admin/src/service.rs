//! The session operations every way in shares: starting, describing and stopping a session, and one agent call with
//! its admission, takeover blocking and redaction. The HTTP handlers authenticate their caller and map these outcomes to
//! their own responses; none calls the agent directly.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use display_protocol::session::{MAX_URL, start_url};
use egress::check::CheckOutcome;
use serde_json::{Value, json};

use crate::agent;
use crate::app::{self, App, Session};
use crate::coded::{Code, Coded};
use crate::controller::NewSession;
use crate::redact::Scrub;

/// How long a new session's display has to open its stream socket, and how often the admin looks for it.
const DISPLAY_START: Duration = Duration::from_secs(10);
const DISPLAY_POLL: Duration = Duration::from_millis(100);

/// Who made a call, as a timeline names them: `via` `token` (and the token's name), `local` (the local MCP clients) or
/// `admin` (the administrator's page).
pub fn caller(app: &App, who: app::Owner) -> Value {
    match who {
        app::Owner::Token(id) => json!({"via": "token", "token": app.tokens.lock().unwrap().name(id)}),
        app::Owner::Local => json!({"via": "local"}),
        app::Owner::Admin | app::Owner::Visitor(_) => json!({"via": "admin"}),
    }
}

/// `fields` with who made the call ([`caller`]) added.
pub fn by(mut fields: Value, caller: &Value) -> Value {
    if let (Value::Object(fields), Value::Object(caller)) = (&mut fields, caller) {
        fields.extend(caller.clone());
    }
    fields
}

pub fn describe(session: &Session) -> Value {
    let state = session.state.lock().unwrap();
    json!({
        "id": session.id,
        "url": session.url,
        "devtools": session.devtools,
        "egress": session.egress,
        "egress_id": session.egress_id,
        "profile": session.profile.as_ref().map(|p| json!({"id": p.id, "name": p.name})),
        "features": session.features,
        "viewers": state.viewers.len(),
        "lease_epoch": state.lease_epoch,
        "controlled": state.holder.is_some(),
        "agent_blocked": session.agent.is_blocked(),
        "redactions": state.redactions.len(),
        "redactions_full": state.redactions.saturated,
        "settings": *session.settings.borrow(),
        "network": state.network,
        "failed": state.failed,
        "help": state.help.as_ref().map(|h| json!({"reason": h.reason, "message": h.message, "since": h.since})),
    })
}

/// Where the administrator watches a session: the admin page, which opens that session's viewer once they are logged
/// in.
pub fn watch_url(app: &App, id: &str) -> String {
    format!("{}#session={id}", page(&app.config.origin, &app.config.static_path))
}

/// The admin page's address: the origin and the web client's path (`ADMIN_STATIC_PATH`), with its slash.
fn page(origin: &str, static_path: &str) -> String {
    let page = if static_path == "/" { "/".to_owned() } else { format!("{static_path}/") };
    format!("{origin}{page}")
}

/// The sessions `who` may reach (see [`App::reachable`]).
pub fn sessions_of(app: &App, who: app::Owner) -> Vec<Arc<Session>> {
    app.sessions.lock().unwrap().values().filter(|s| app.reachable(who, s)).cloned().collect()
}

/// A start URL as a caller gave it, trimmed, and checked as the controller checks it again
/// ([`start_url`]).
pub fn target(url: &str) -> Result<String, Coded> {
    let url = url.trim();
    start_url(url).map_err(|why| Coded::new(Code::BadUrl, format!("the URL {why}")).with("max", MAX_URL))?;
    Ok(url.to_owned())
}

/// The egress a start asks for: by its id (the administrator's page) or by its name (agents).
pub enum EgressChoice {
    Id(String),
    Name(String),
}

/// What a new session is started with.
pub struct Start {
    /// The new session's id (see [`session_id`]).
    pub id: String,
    pub url: String,
    pub devtools: bool,
    pub egress: Option<EgressChoice>,
    pub kiosk: bool,
    pub lane: Option<u8>,
    pub owner: app::Owner,
    pub features: app::Features,
    /// The id of a saved profile to start from, which `owner` must be allowed to use (see `profiles`).
    pub profile: Option<String>,
    /// Its screen's size, as the window that will watch it asks (within what the protocol carries); the configured
    /// size if none.
    pub size: Option<(u32, u32)>,
    /// Its browser's languages, in place of the installation's (or its egress's): a demo visitor's own.
    pub languages: Option<Vec<String>>,
}

/// Why a start was refused or failed: its status, and what it says (see `coded`): a failed check of its egress, by
/// the check's code, with its `detail`.
#[derive(Clone, Debug)]
pub struct Refused {
    pub status: StatusCode,
    pub why: Coded,
}

impl From<(StatusCode, Coded)> for Refused {
    fn from((status, why): (StatusCode, Coded)) -> Refused {
        Refused { status, why }
    }
}

impl Refused {
    /// As JSON: `{"code", "error", ...fields}`.
    pub fn response(self) -> Response {
        crate::api::coded(self.status, self.why)
    }
}

/// A new session id, of six random bytes.
pub fn session_id() -> String {
    display_protocol::session::SessionId::from_bytes(rand::random()).to_string()
}

/// Starts a session and lists it once its display is up, and its saved profile, if it starts from one, is restored; a
/// session that fails to start, or whose display never comes up, is undone.
pub async fn start(app: &Arc<App>, start: Start) -> Result<Arc<Session>, Refused> {
    let Start { id, url, devtools, egress, kiosk, lane, owner, features, profile, size, languages } = start;
    let Some(reservation) = app::reserve(app) else {
        if app.stopping.load(std::sync::atomic::Ordering::SeqCst) {
            let why = "the admin is stopping (an update or a restart); try again once it is back";
            return Err((StatusCode::SERVICE_UNAVAILABLE, Coded::new(Code::AdminStopping, why)).into());
        }
        return Err(
            (StatusCode::CONFLICT, Coded::new(Code::TooManySessions, "too many sessions; stop one first")).into()
        );
    };
    // Taken for this session now, so no other starts from it meanwhile; given back on every way out before the session
    // is listed, and by its stop after.
    let profile = profile.map(|profile| {
        let kek = app.auth.lock().unwrap().kek();
        app.profiles.lock().unwrap().reserve(owner, &profile, &id, kek.as_ref())
    });
    let profile = profile.transpose().map_err(|refused| {
        // An agent cannot go on until the administrator logs in: they are told (see `notify`).
        if refused == crate::profiles::Refused::Locked && owner != app::Owner::Admin {
            app.notify.send(crate::notify::Event::UnlockNeeded, None);
        }
        refused.answer()
    })?;
    // Taken for this session as it is now, so it cannot be deleted meanwhile; given back as the profile is.
    let egress = egress.map(|choice| {
        let (key, by_name) = match &choice {
            EgressChoice::Id(id) => (id, false),
            EgressChoice::Name(name) => (name, true),
        };
        app.egresses.lock().unwrap().reserve(key, by_name, &id)
    });
    let egress = match egress.transpose() {
        Ok(egress) => egress,
        Err(_) => {
            if let Some(profile) = &profile {
                app.profiles.lock().unwrap().release(&profile.id, &id);
            }
            return Err((StatusCode::BAD_REQUEST, Coded::new(Code::NoProxy, "no such egress")).into());
        }
    };
    let release = || {
        if let Some(profile) = &profile {
            app.profiles.lock().unwrap().release(&profile.id, &id);
        }
        app.egresses.lock().unwrap().release(&id);
    };
    // Through an egress, the upstream's user name carries the session's id for providers that keep one exit address per
    // such session; that very upstream is checked before anything of the session exists, unless it is the same for
    // every session and passed a check just now.
    // A saved session's own id stands for it there across its resumes, so such a provider keeps it at the same address.
    let route = profile.as_ref().map_or(id.as_str(), |p| p.id.as_str());
    let upstream = match egress.as_ref().map(|e| e.upstream.for_session(route)).transpose() {
        Ok(upstream) => upstream,
        Err(why) => {
            release();
            let why = Coded::new(Code::ServerFailed, format!("the egress: {why}"));
            return Err((StatusCode::INTERNAL_SERVER_ERROR, why).into());
        }
    };
    if let (Some(egress), Some(upstream)) = (&egress, &upstream)
        && !app.egresses.lock().unwrap().passed(egress)
    {
        let began = Instant::now();
        let checked = crate::egresses::check(&app.controller, upstream).await;
        let refused = match &checked {
            Ok(checked) => {
                app.egresses.lock().unwrap().record(egress, began, checked);
                match checked {
                    CheckOutcome::Passed { .. } => None,
                    CheckOutcome::Failed { reason, detail } => Some(Refused {
                        status: StatusCode::FAILED_DEPENDENCY,
                        why: Coded::new(
                            Code::of_check(*reason),
                            format!("the egress {} failed its check: {detail}", egress.name),
                        )
                        .with("detail", detail.clone())
                        .with("name", egress.name.clone()),
                    }),
                }
            }
            Err(unchecked) => Some(unchecked.answer().into()),
        };
        if let Some(refused) = refused {
            release();
            tracing::info!(session = %id, "a start was refused: its egress did not pass its check");
            return Err(refused);
        }
    }
    let dir = app.config.streams_dir.join(&id);
    if let Err(e) = std::fs::create_dir(&dir) {
        release();
        return Err((StatusCode::INTERNAL_SERVER_ERROR, Coded::new(Code::ServerFailed, e.to_string())).into());
    }
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o770));
    let size = size.unwrap_or((app.config.width, app.config.height));
    // Through an egress, the browser says it is where that egress leaves; it speaks the languages given, if any.
    let mut locale = egress.as_ref().map_or(&app.config.locale, |e| &e.locale).clone();
    if let Some(languages) = languages {
        locale.languages = languages;
    }
    let walled = app.config.demo.is_some();
    let new = NewSession {
        session: &id,
        url: &url,
        devtools,
        size,
        locale: &locale,
        lane,
        egress: upstream.as_ref(),
        kiosk,
        walled,
        profile: profile.is_some(),
    };
    let failed = match app.controller.create_session(&new).await {
        // The controller may still hold what it built: a rollback it could not complete, or a build it finishes after
        // this gave up waiting. It is stopped like any failed session.
        Err(e) => Some(Coded::new(Code::ServerFailed, e)),
        Ok(()) => {
            let socket = dir.join("display.sock");
            let deadline = Instant::now() + DISPLAY_START;
            let mut ready = socket.exists();
            while !ready && Instant::now() < deadline {
                tokio::time::sleep(DISPLAY_POLL).await;
                ready = socket.exists();
            }
            // A display that never opened its stream socket cannot be watched.
            if !ready {
                tracing::warn!(session = %id, timeout = ?DISPLAY_START, "the display did not open its stream socket");
                Some(Coded::new(Code::DisplayNotStarted, "the display did not start"))
            } else if let Some(profile) = &profile {
                // Chrome starts once its profile is in place.
                crate::profiles::restore(app, &id, profile).await.err().inspect(|why| {
                    // Escaped: the browser's part of it may name a file a page chose.
                    tracing::warn!(session = %id, ?why, "the session's profile was not restored");
                })
            } else {
                None
            }
        }
    };
    let name = egress.as_ref().map(|e| e.name.clone());
    let agent = agent::Agent::new(id.clone(), app.controller.clone(), dir.join("display.sock"));
    let mut session = Session::new(id.clone(), url, devtools, name, owner, features, agent);
    // An agent's start (through the agent API or MCP) gives it control: a person who opens it watches until they take
    // over (see `Session::admit`).
    if matches!(owner, app::Owner::Token(_) | app::Owner::Local) {
        session.started_by_agent();
    }
    session.profile = profile;
    session.egress_id = egress.map(|e| e.id);
    // Its agent's screen, recorded for the administrator's replay unless the session says not to (never in the demo).
    if session.features.replay && app.config.demo.is_none() {
        match crate::replay::Recording::new(&app.config.state_dir, &id) {
            Ok(recording) => {
                let recording = Arc::new(recording);
                app.replays.add(recording.clone());
                session.recording = Some(recording);
            }
            Err(e) => tracing::warn!(session = %id, %e, "the session's replay cannot be recorded"),
        }
    }
    let session = Arc::new(session);
    // Its place passes to the listing under the sessions lock, so it is never counted twice. Listed only if its owner
    // may still act: a token revoked while this started never gets it. The check and the listing happen under the
    // sessions lock, which the revocation's search for the token's sessions takes after it marked the token revoked,
    // so one of them always finds the session.
    let allowed = {
        let mut sessions = app.sessions.lock().unwrap();
        sessions.insert(id.clone(), session.clone());
        drop(reservation);
        app.may_act(owner)
    };
    if let Some(why) = failed {
        // Undone rather than kept. If the undo fails, the session stays listed as failed, so stopping it can be
        // retried.
        session.state.lock().unwrap().failed = Some(why.clone());
        return Err(match stop(app, &session).await {
            Ok(()) => (StatusCode::BAD_GATEWAY, why),
            Err(e) => {
                let told = format!("{why}, and stopping it failed: {e}");
                (StatusCode::BAD_GATEWAY, why.told(told))
            }
        }
        .into());
    }
    if !allowed {
        tracing::info!(session = %id, "its token was revoked while it started; stopping it");
        session.state.lock().unwrap().failed = Some(Coded::new(Code::TokenRevoked, "its token was revoked"));
        let _ = stop(app, &session).await;
        return Err((StatusCode::UNAUTHORIZED, Coded::new(Code::TokenRevoked, "the token was revoked")).into());
    }
    // A saved session remembers where it left from, so a resume that names no egress leaves from there again.
    if let Some(profile) = &session.profile {
        app.profiles.lock().unwrap().left_through(&profile.id, session.egress_id.as_deref());
    }
    if app.config.demo.is_none() {
        crate::agent_files::spawn(app.clone(), session.clone());
    }
    if let Some(recording) = &session.recording {
        crate::replay::spawn(app.clone(), session.clone(), recording.clone());
    }
    let by = if matches!(session.owner, app::Owner::Admin) { "person" } else { "agent" };
    if app.config.demo.is_some() {
        session.agent.timeline.off();
    }
    session.agent.timeline.push("session", json!({"state": "started", "by": by}));
    tracing::info!(session = %id, devtools, "session started");
    Ok(session)
}

pub async fn stop(app: &App, session: &Session) -> Result<(), String> {
    {
        let mut state = session.state.lock().unwrap();
        // Ended under the state lock, before the revocation: no grant comes after it (see `Session::admit`). A relay
        // that cannot take the close (blocked on a viewer that stopped reading) ends anyway.
        session.ended.send_replace(true);
        Session::set_holder(&mut state, None);
        for viewer in state.viewers.values() {
            let _ = viewer.tx.try_send(app::ToViewer::Close("the session ended"));
        }
    }
    // Its agent link gives its controller connection back first, so the stop itself finds one free.
    session.agent.detach();
    // Its profile is saved while its browser still runs; a profile that was never restored is not.
    if let Some(profile) = &session.profile {
        crate::profiles::save_at_end(app, &session.id, profile).await;
    }
    // A session the controller did not stop stays listed, marked failed, so stopping it can be retried.
    if let Err(e) = app.controller.stop_session(&session.id).await {
        session.state.lock().unwrap().failed = Some(Coded::new(Code::StopFailed, format!("stopping failed: {e}")));
        return Err(e);
    }
    app.sessions.lock().unwrap().remove(&session.id);
    keep_timeline(app, session).await;
    if let Some(profile) = &session.profile {
        app.profiles.lock().unwrap().release(&profile.id, &session.id);
    }
    app.egresses.lock().unwrap().release(&session.id);
    let _ = std::fs::remove_dir_all(app.config.streams_dir.join(&session.id));
    Ok(())
}

/// How long the admin, stopped, takes at most to save its persistent sessions, all of it (see [`save_before_exit`]);
/// compose.yaml gives it a little more before Docker kills it.
pub const SAVE_BEFORE_EXIT: Duration = Duration::from_secs(110);

/// Stops every session that runs from a saved one, all at once, each saved as its stop saves it, within
/// [`SAVE_BEFORE_EXIT`]: what the admin does as it is stopped, so an update or a restart keeps what was done in them.
pub async fn save_before_exit(app: &Arc<App>) {
    {
        let _sessions = app.sessions.lock().unwrap();
        app.stopping.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    // One deadline for all of it. Starts already under way get a few seconds to be listed; one still restoring its
    // profile after that has done nothing in it yet, so the archive it started from stays as it was.
    let deadline = tokio::time::Instant::now() + SAVE_BEFORE_EXIT;
    let settled = async {
        while app.starting.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(10), settled).await;
    let persistent: Vec<Arc<Session>> =
        app.sessions.lock().unwrap().values().filter(|s| s.profile.is_some()).cloned().collect();
    if persistent.is_empty() {
        return;
    }
    tracing::info!(sessions = persistent.len(), "stopping: saving the persistent sessions first");
    let mut stops = tokio::task::JoinSet::new();
    for session in persistent {
        let app = app.clone();
        stops.spawn(async move { stop(&app, &session).await });
    }
    if tokio::time::timeout_at(deadline, stops.join_all()).await.is_err() {
        tracing::warn!("stopping: not every persistent session was saved in time; those keep their last save");
    }
}

/// Keeps a stopped session's timeline, scrubbed with its redactions as they stand, for the administrator to read (see
/// `timeline::Runs`); never in the demo.
async fn keep_timeline(app: &App, session: &Session) {
    if app.config.demo.is_some() {
        return;
    }
    session.agent.timeline.push("session", json!({"state": "stopped"}));
    let mut page = session.agent.timeline.page(0);
    page["label"] = json!(label(session));
    app.runs.keep(crate::timeline::Run {
        id: session.id.clone(),
        stopped: crate::timeline::now_ms(),
        until: std::time::Instant::now() + crate::timeline::RETAINED_FOR,
        page: timeline_scrubbed(session, page).await,
        recording: session.recording.clone(),
    });
}

/// What a timeline calls its session: its saved session's name, or the page it started at.
pub fn label(session: &Session) -> String {
    session.profile.as_ref().map_or_else(|| session.url.clone(), |p| p.name.clone())
}

/// A page of `session`'s timeline as an answer gives it: its texts that can quote what a person typed scrubbed with
/// the session's redactions, then cut (see `timeline::put_texts`); its structure is never scrubbed.
pub async fn timeline_scrubbed(session: &Session, mut page: Value) -> Value {
    let texts = json!(crate::timeline::texts(&page));
    let scrubbed = redacted(session, texts, None).await.and_then(|(texts, _)| serde_json::from_value(texts).ok());
    crate::timeline::put_texts(&mut page, scrubbed);
    page
}

/// How one agent call ended.
#[derive(Debug)]
pub enum Called {
    /// The tool's MCP result, with the redaction contract applied.
    Done(Value),
    /// The session was stopped after a failure.
    Stopped,
    /// A person holds control (or a takeover is under way).
    Controlled,
    /// The caller's token was revoked while the call waited for its turn.
    Revoked,
    /// The caller no longer reaches the session (see [`App::reachable`]): the saved profile it reached it through
    /// stopped allowing it while the call waited for its turn.
    Unreached,
    /// Too many calls wait for this session's agent already, or this one waited too long; it asked nothing.
    Busy,
    Failed(String),
}

/// One tool call on a session's agent for `who`, checked again once the agent admits it (see [`App::may_act`],
/// [`App::reachable`]), and what its answer holds of the agents' memory budget, to give back with the response made of
/// it (see [`respond`]).
/// The agent runs an admitted call as its own task (see `Agent::call`): a caller that disconnects cannot cut it.
pub async fn call(
    app: &App,
    who: app::Owner,
    session: &Session,
    tool: &str,
    arguments: Value,
    call_id: Option<&str>,
) -> (Called, Option<agent::Held>) {
    if session.state.lock().unwrap().failed.is_some() {
        return (Called::Stopped, None);
    }
    let arguments = if arguments.is_null() { json!({}) } else { arguments };
    if let Some(id) = call_id {
        if !crate::tools::known(tool) {
            return (Called::Failed("`call_id` is for the desktop tools only".into()), None);
        }
        if !crate::calls::valid_id(id) {
            let why = format!("`call_id` is 1 to {} letters, digits, - or _", crate::calls::MAX_ID);
            return (Called::Failed(why), None);
        }
    }
    if crate::person::known(tool) {
        return (crate::person::call(app, who, session, tool, arguments).await, None);
    }
    let caller = caller(app, who);
    if crate::agent_files::known(tool) {
        use crate::timeline::call_event;
        // Recorded however the call ends: its caller may leave while it waits for the administrator's answer.
        struct Recording<'a> {
            session: &'a Session,
            event: Option<Value>,
            tool: &'a str,
            action: Value,
            started: std::time::Instant,
            caller: Value,
        }
        impl Drop for Recording<'_> {
            fn drop(&mut self) {
                let left = || call_event(self.tool, &self.action, "left", Some(self.started.elapsed()));
                let event = self.event.take().unwrap_or_else(left);
                self.session.agent.timeline.push("call", by(event, &self.caller));
            }
        }
        let action = crate::timeline::summary(tool, &arguments);
        let started = std::time::Instant::now();
        let mut recording = Recording { session, event: None, tool, action, started, caller };
        let mut called = crate::agent_files::call(app, who, session, tool, arguments).await;
        if let Called::Done(result) = &mut called {
            told_of_person(session, result);
        }
        let (took, action) = (Some(recording.started.elapsed()), &recording.action);
        recording.event = Some(match &called {
            Called::Done(result) if crate::timeline::tool_failed(result) => call_event(tool, action, "error", took),
            Called::Done(_) => call_event(tool, action, "ok", took),
            Called::Failed(_) => call_event(tool, action, "failed", took),
            Called::Controlled => call_event(tool, action, "blocked", None),
            Called::Busy => call_event(tool, action, "busy", None),
            _ => call_event(tool, action, "withdrawn", None),
        });
        return (called, None);
    }
    let allowed = || app.may_act(who) && app.reachable(who, session);
    // Its receipt is taken before it waits for its turn: the same call sent again meanwhile finds it (see `calls`).
    let receipt = match call_id {
        None => None,
        Some(id) => {
            use crate::calls::Known;
            let fingerprint = crate::calls::fingerprint(tool, &arguments);
            match session.agent.calls.reserve(id, fingerprint, tool, &caller) {
                Ok(receipt) => Some(receipt),
                Err(Known::Same(receipt)) => return (Called::Done(repeated(receipt)), None),
                Err(Known::Other) => {
                    let why =
                        "this `call_id` named another call (another tool or other arguments); give this one its own";
                    return (Called::Failed(why.into()), None);
                }
                Err(Known::Full) => {
                    let why = format!(
                        "this session has used {} call ids, all it keeps; call without one, or start another session",
                        crate::calls::MAX_RECEIPTS
                    );
                    return (Called::Failed(why), None);
                }
            }
        }
    };
    // The call's place is kept until its result or error is scrubbed (see `Agent::place`).
    let Some(mut turn) = session.agent.place() else {
        let action = crate::timeline::summary(tool, &arguments);
        let mut event = crate::timeline::call_event(tool, &action, "busy", None);
        if let Some(id) = call_id {
            event["call_id"] = json!(id);
        }
        session.agent.timeline.push("call", by(event, &caller));
        return (Called::Busy, None);
    };
    let outcome = session.agent.call(&mut turn, tool, arguments, caller, receipt, allowed).await;
    let held = turn.answer();
    let (value, done) = match outcome {
        Ok(result) => (result, true),
        // What the driver said went wrong, which may quote the page as well.
        Err(agent::CallError::Failed(why)) => (Value::String(why), false),
        Err(agent::CallError::Blocked) => return (Called::Controlled, held),
        Err(agent::CallError::Busy) => return (Called::Busy, held),
        Err(agent::CallError::Withdrawn) if app.may_act(who) => return (Called::Unreached, held),
        Err(agent::CallError::Withdrawn) => return (Called::Revoked, held),
    };
    let Some((mut value, held)) = redacted(session, value, held).await else {
        let why = "the agent's answer would take more memory than is free once redacted";
        return (Called::Failed(why.into()), None);
    };
    if done {
        told_of_person(session, &mut value);
    }
    (if done { Called::Done(value) } else { Called::Failed(text(value)) }, held)
}

/// The answer to a call whose `call_id` named this very call before: its receipt, as a tool result, and that it was
/// not made again (see `calls`).
fn repeated(receipt: Value) -> Value {
    let text = match receipt["state"].as_str() {
        Some("waiting" | "running") => "This call_id names a call that is still running, so it was not sent again. \
            call_status says how it ends; then read the window again (get_window_state) before acting."
            .to_owned(),
        state => format!(
            "This call_id names a call that ran already (it ended: {}), so it was not run again, and its result is not \
             kept. Read the window again (get_window_state) before acting.",
            state.unwrap_or("unknown")
        ),
    };
    json!({"content": [{"type": "text", "text": text}, {"type": "text", "text": receipt.to_string()}], "isError": false})
}

/// What the agent's first result after a person held control adds (see [`told_of_person`]).
pub const PERSON_ACTED: &str = "Note: a person held control of this session since your last call and may have \
    changed the page. Read the window again (get_window_state) before acting on what you saw before.";

/// Adds [`PERSON_ACTED`] to a tool's result once after a person held control, and forgets it.
fn told_of_person(session: &Session, result: &mut Value) {
    let acted = {
        let mut state = session.state.lock().unwrap();
        // Told once the agent holds control again; a result that comes back while a person holds it keeps it.
        state.holder.is_none() && std::mem::take(&mut state.person_acted)
    };
    if let (true, Some(content)) = (acted, result.get_mut("content").and_then(Value::as_array_mut)) {
        content.push(json!({"type": "text", "text": PERSON_ACTED}));
    }
}

/// `body` as a JSON response. `held`, what the agent's answer it was made of holds of the agents' budget, comes down to
/// what the response's bytes take once `body` is serialized and dropped, and goes back with those bytes: once hyper has
/// written the last of them, or dropped them with their connection (see `agent::ANSWERS`), which a client that stops
/// reading keeps only until a write to it has waited past the deadline (see `deadline`).
pub fn respond(status: StatusCode, body: Value, held: Option<agent::Held>) -> Response {
    let (bytes, held) = encoded(body, false, held);
    // Hyper keeps the body's bytes themselves, never a copy, until they are written: the share goes with them.
    let body = Body::from(Bytes::from_owner(Answer { bytes, _held: held }));
    (status, [(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))], body).into_response()
}

/// `body` as JSON bytes, followed by a line feed when `line` says so (as the local socket writes its messages), and what
/// `held`, the share of the agents' budget of the agent's answer it was made of, comes down to once `body` is dropped:
/// what those bytes take (see `agent::serialized`), to go with them until they are written.
pub fn encoded(body: Value, line: bool, held: Option<agent::Held>) -> (Vec<u8>, Option<agent::Held>) {
    let mut bytes = serde_json::to_vec(&body).expect("JSON values encode");
    drop(body);
    if line {
        bytes.push(b'\n');
    }
    // Kept at its length: `to_vec` writes into a buffer that doubles as it grows and keeps all of it, which made a
    // 10.6 MiB answer take 18.3 MiB. glibc shrinks a block that large in place, without copying it.
    bytes.shrink_to_fit();
    let held = held.map(|held| agent::serialized(held, bytes.len()));
    (bytes, held)
}

/// A response's bytes and what they hold of the agents' budget, given back when the last reference to them is dropped.
struct Answer {
    bytes: Vec<u8>,
    _held: Option<agent::Held>,
}

impl AsRef<[u8]> for Answer {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

/// The one permit to scrub an agent result (see [`redacted`]).
static SCRUBBING: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

/// `value` with the session's redaction contract applied, its matcher built, off the async workers and outside the
/// session's lock but for taking the contract and keeping the matcher built for the next call: a full set takes tens
/// of milliseconds to build and a result of tens of MiB takes seconds to scrub, while every session's relay, login and
/// API request share those workers (a single one under the admin's CPU quota), and every viewer's relay takes that lock
/// for each message.
///
/// One result is scrubbed at a time, all sessions together: what scrubbing takes besides the result (the matcher, the
/// scrubbed copy) is held once, and the admin's one CPU does one at a time anyway. The permit goes with the work, so a
/// caller that leaves frees it only once that work is done; so does `held`, what the value holds of the agents' budget,
/// which comes back with it grown to what the scrubbed value takes as JSON (see `Held::grown`), or none at all, with
/// no value, when that growth is not free.
pub async fn redacted(
    session: &Session,
    value: Value,
    held: Option<agent::Held>,
) -> Option<(Value, Option<agent::Held>)> {
    let permit = SCRUBBING.acquire().await.expect("never closed");
    let scrub = session.state.lock().unwrap().redactions.scrub();
    let (scrub, built, value, held, length) = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let (scrub, built, value) = scrubbed(scrub, value);
        // Counted only for a value that holds a share, in one pass that writes nothing.
        let length = held.as_ref().map(|_| encoded_length(&value));
        (scrub, built, value, held, length)
    })
    .await
    .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()));
    if built {
        session.state.lock().unwrap().redactions.install(&scrub);
    }
    let held = match held.zip(length) {
        Some((held, length)) => Some(held.grown(length)?),
        None => None,
    };
    Some((value, held))
}

/// How many bytes `value` takes as JSON, counted without writing them anywhere.
fn encoded_length(value: &Value) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value).expect("JSON values encode");
    count.0
}

/// `value` with `scrub` applied, and whether its matcher was built for it.
fn scrubbed(mut scrub: Scrub, mut value: Value) -> (Scrub, bool, Value) {
    let built = scrub.build();
    scrub.apply(&mut value);
    (scrub, built, value)
}

/// A scrubbed string as text: scrubbing a string leaves a string. The driver's error comes as JSON text, in which a
/// recorded string is found escaped too (see `Scrub::scrub`).
fn text(value: Value) -> String {
    let Value::String(text) = value else { unreachable!("a scrubbed string is a string") };
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_url_is_refused_here_as_the_controller_would_refuse_it() {
        assert_eq!(target(" https://example.com/ ").unwrap(), "https://example.com/");
        assert_eq!(target("http://user@[::1]:8080/?q=x").unwrap(), "http://user@[::1]:8080/?q=x");
        let hostless = ["https://", "http:///x", "https://?q=x", "https://#top", "https://user@/x", "https://:443/"];
        let malformed = ["ftp://e.test", "https://e.test/a b", "https://e.test/\u{7f}"];
        for bad in hostless.into_iter().chain(malformed) {
            let why = target(bad).unwrap_err();
            assert_eq!((why.code, why.json()["max"].clone()), (Code::BadUrl, json!(MAX_URL)), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn one_result_is_scrubbed_at_a_time() {
        let session = Arc::new(Session::test("abc", app::Owner::Admin, app::Features::default()));
        session.state.lock().unwrap().redactions.keep("hunter2-secret");
        let held = SCRUBBING.acquire().await.unwrap();
        let waiting = tokio::spawn({
            let session = session.clone();
            async move { redacted(&session, json!("a hunter2-secret"), None).await.unwrap().0 }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished(), "it waits for the permit");
        drop(held);
        assert_eq!(waiting.await.unwrap(), "a [redacted]");
        assert!(!session.state.lock().unwrap().redactions.scrub().build(), "its matcher is kept");
    }

    #[tokio::test]
    async fn a_scrubbed_answer_holds_what_its_bytes_take_or_fails_when_that_is_not_free() {
        let budget: &'static agent::Budget = Box::leak(Box::new(agent::Budget::new(1000)));
        let mut session = Session::test("abc", app::Owner::Admin, app::Features::default());
        session.agent = agent::Agent::on(
            "abc".into(),
            crate::controller::Controller::new("/nonexistent".into()),
            "/nonexistent".into(),
            budget,
        );
        session.state.lock().unwrap().redactions.keep("abcd");
        // Some 100 KiB of text, a recorded value every five bytes: more than twice as long once scrubbed.
        let text = || json!("abcd ".repeat(20 * 1024));
        let (value, held) = redacted(&session, text(), Some(budget.take(110).unwrap())).await.unwrap();
        let length = serde_json::to_vec(&value).unwrap().len();
        assert!(length > 2 * 100 * 1024, "{length}");
        assert_eq!(held.as_ref().unwrap().kib(), length.div_ceil(1024), "the growth is held too");
        let response = respond(StatusCode::OK, value, held);
        assert_eq!(budget.free(), 1000 - length.div_ceil(1024));
        drop(response);
        // With less free than the growth, the answer goes, and so does its share.
        let others = budget.take(800).unwrap();
        assert!(redacted(&session, text(), Some(budget.take(110).unwrap())).await.is_none());
        assert_eq!(budget.free(), 200);
        drop(others);
    }

    #[tokio::test]
    async fn a_response_is_its_body_s_json() {
        let response = respond(StatusCode::BAD_GATEWAY, json!({"error": "a \"quoted\" why"}), None);
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], br#"{"error":"a \"quoted\" why"}"#);
    }

    #[tokio::test]
    async fn an_answer_holds_what_its_bytes_take_of_the_budget_until_the_last_of_them_is_dropped() {
        use axum::body::HttpBody;
        let budget: &'static agent::Budget = Box::leak(Box::new(agent::Budget::new(1000)));
        let held = budget.take(100).unwrap();
        // A little over 10 KiB once serialized.
        let response = respond(StatusCode::OK, json!({"text": "a".repeat(10 * 1024)}), Some(held));
        assert_eq!(budget.free(), 1000 - 11, "the value is gone, its bytes keep their share");
        let mut body = response.into_body();
        let frame = std::future::poll_fn(|cx| std::pin::Pin::new(&mut body).poll_frame(cx)).await.unwrap().unwrap();
        let bytes = frame.into_data().unwrap();
        drop(body);
        assert_eq!(budget.free(), 1000 - 11, "bytes taken from the body but not yet written still count");
        assert_eq!(bytes.len(), 10 * 1024 + 11);
        drop(bytes);
        assert_eq!(budget.free(), 1000);
    }

    #[tokio::test]
    async fn a_call_queued_before_its_saved_session_stops_allowing_its_caller_is_unreached_and_asks_nothing() {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let (controller, ops, _dir) = agent::tests::fake_controller("unreached", Some(barrier.clone())).await;
        let agent = agent::Agent::new("abc".into(), controller, "/nonexistent/display.sock".into());
        let app = app::test_app(false);
        let kek = crate::keys::derive("correct horse battery", &[1; crate::keys::SALT]);
        let id = app.profiles.lock().unwrap().create("x.com", Some(&kek)).unwrap()["id"].as_str().unwrap().to_owned();
        let allow = |allowed: &[String]| app.profiles.lock().unwrap().update(&id, None, Some(allowed)).unwrap();
        allow(&["local".into()]);
        // The person resumed it from the admin page.
        let url = "https://example.com/".to_owned();
        let mut session =
            Session::new("abc".into(), url, false, None, app::Owner::Admin, app::Features::default(), agent);
        let reserved = app.profiles.lock().unwrap().reserve(app::Owner::Admin, &id, "abc", Some(&kek));
        session.profile = Some(reserved.unwrap());
        let session = Arc::new(session);
        let calling = || {
            let (app, session) = (Arc::clone(&*app), session.clone());
            tokio::spawn(async move { call(&app, app::Owner::Local, &session, "click", json!({}), None).await.0 })
        };
        // The first call holds the agent while the driver works on it; the second waits for its turn behind it.
        let first = calling();
        let asked = agent::tests::recorded(&ops, "tools/call");
        tokio::time::timeout(Duration::from_secs(10), asked).await.expect("the first call was asked");
        let second = calling();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!second.is_finished(), "it waits for its turn");
        allow(&[]);
        barrier.wait().await;
        assert!(matches!(first.await.unwrap(), Called::Done(_)), "the call admitted before goes on");
        let second = tokio::time::timeout(Duration::from_secs(10), second).await.expect("answered at once");
        assert!(matches!(second.unwrap(), Called::Unreached));
        assert_eq!(ops.lock().unwrap().iter().filter(|o| *o == "tools/call").count(), 1, "the driver was asked once");
    }

    #[tokio::test]
    async fn a_call_sent_again_with_its_call_id_is_answered_with_how_the_first_went_and_made_once() {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let (controller, ops, _dir) = agent::tests::fake_controller("again", Some(barrier.clone())).await;
        let agent = agent::Agent::new("abc".into(), controller, "/nonexistent/display.sock".into());
        let app = app::test_app(false);
        let url = "https://example.com/".to_owned();
        let session = Session::new("abc".into(), url, false, None, app::Owner::Local, app::Features::default(), agent);
        let session = Arc::new(session);
        let calling = |arguments: Value, id: Option<&'static str>| {
            let (app, session) = (Arc::clone(&*app), session.clone());
            tokio::spawn(async move { call(&app, app::Owner::Local, &session, "click", arguments, id).await.0 })
        };
        let said = |called: Called| match called {
            Called::Done(result) => result["content"][0]["text"].as_str().unwrap_or_default().to_owned(),
            Called::Failed(why) => why,
            _ => panic!("answered"),
        };
        let asked = || ops.lock().unwrap().iter().filter(|o| *o == "tools/call").count();
        // The first holds the driver; the same call sent again meanwhile is not made, and says it runs.
        let first = calling(json!({"x": 1, "y": 2}), Some("buy"));
        let reached = agent::tests::recorded(&ops, "tools/call");
        tokio::time::timeout(Duration::from_secs(10), reached).await.expect("the first call was asked");
        assert_eq!(session.agent.calls.running()[0]["call_id"], "buy", "listed as running");
        let again = calling(json!({"y": 2, "x": 1}), Some("buy")).await.unwrap();
        assert!(said(again).contains("still running"));
        let other = calling(json!({"x": 9}), Some("buy")).await.unwrap();
        assert!(said(other).contains("another call"));
        barrier.wait().await;
        assert!(matches!(first.await.unwrap(), Called::Done(_)));
        // Once it ended: how it went, still not made again.
        let after = calling(json!({"x": 1, "y": 2}), Some("buy")).await.unwrap();
        assert!(said(after).contains("ended: ok"), "said how it went");
        assert_eq!(asked(), 1, "the driver was asked once");
        assert_eq!(session.agent.calls.receipt("buy").unwrap()["state"], "ok");
        assert!(session.agent.calls.running().is_empty());
        let recent = session.agent.timeline.page(0)["events"].clone();
        assert_eq!(
            recent.as_array().unwrap().iter().filter(|e| e["call_id"] == "buy").count(),
            1,
            "one call, recorded once"
        );
        // Ids are checked, and only the desktop tools take one.
        let bad = call(&app, app::Owner::Local, &session, "click", json!({}), Some("not ok")).await.0;
        assert!(said(bad).contains("letters, digits"));
        let person = call(&app, app::Owner::Local, &session, "wait_for_control", json!({}), Some("w")).await.0;
        assert!(said(person).contains("desktop tools only"));
    }

    #[test]
    fn a_driver_error_loses_what_the_person_typed_even_when_escaped() {
        let mut redactions = crate::redact::Redactions::default();
        redactions.keep("pa\"ss\\word");
        let shown = |why: String| text(scrubbed(redactions.scrub(), Value::String(why)).2);
        let error = json!({"code": -32000, "message": "no field named pa\"ss\\word"}).to_string();
        let error = shown(error);
        assert!(!error.contains("word") && error.contains("[redacted]"), "{error}");
        assert_eq!(shown("agent: pa\"ss\\word".into()), "agent: [redacted]");
    }
}
