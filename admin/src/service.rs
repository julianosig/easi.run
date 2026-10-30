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
}

/// Why a start was refused or failed: its status, and what it says (see `coded`): a failed check of its egress, by
/// the check's code, with its `detail`.
#[derive(Debug)]
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
    let Start { id, url, devtools, egress, kiosk, lane, owner, features, profile, size } = start;
    let Some(reservation) = app::reserve(app) else {
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
    let profile = profile.transpose().map_err(|r| r.answer())?;
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
    // Through an egress, the browser says it is where that egress leaves.
    let locale = egress.as_ref().map_or(&app.config.locale, |e| &e.locale);
    let walled = app.config.demo.is_some();
    let new = NewSession {
        session: &id,
        url: &url,
        devtools,
        size,
        locale,
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
    // An agent's start (through the agent API or MCP) is its first use of the session: a person who opens it next
    // watches the agent at work (see `Session::admit`).
    if matches!(owner, app::Owner::Token(_) | app::Owner::Local) {
        session.agent.used_now();
    }
    session.profile = profile;
    session.egress_id = egress.map(|e| e.id);
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
    if let Some(profile) = &session.profile {
        app.profiles.lock().unwrap().release(&profile.id, &session.id);
    }
    app.egresses.lock().unwrap().release(&session.id);
    let _ = std::fs::remove_dir_all(app.config.streams_dir.join(&session.id));
    Ok(())
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
) -> (Called, Option<agent::Held>) {
    if session.state.lock().unwrap().failed.is_some() {
        return (Called::Stopped, None);
    }
    let arguments = if arguments.is_null() { json!({}) } else { arguments };
    let allowed = || app.may_act(who) && app.reachable(who, session);
    // The call's place is kept until its result or error is scrubbed (see `Agent::place`).
    let Some(mut turn) = session.agent.place() else { return (Called::Busy, None) };
    let outcome = session.agent.call(&mut turn, tool, arguments, allowed).await;
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
    let Some((value, held)) = redacted(session, value, held).await else {
        let why = "the agent's answer would take more memory than is free once redacted";
        return (Called::Failed(why.into()), None);
    };
    (if done { Called::Done(value) } else { Called::Failed(text(value)) }, held)
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
async fn redacted(session: &Session, value: Value, held: Option<agent::Held>) -> Option<(Value, Option<agent::Held>)> {
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
            tokio::spawn(async move { call(&app, app::Owner::Local, &session, "click", json!({})).await.0 })
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
