//! The session operations every way in shares: starting, describing and stopping a session, and one agent call with
//! its admission, takeover blocking and redaction. The HTTP handlers authenticate their caller and map these outcomes to
//! their own responses; none calls the agent directly.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::agent;
use crate::app::{self, App, Reservation, Session};
use crate::controller::NewSession;
use crate::redact::Scrub;

/// Longest start URL, in bytes.
const MAX_URL: usize = 2048;
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
        "features": session.features,
        "viewers": state.viewers.len(),
        "lease_epoch": state.lease_epoch,
        "controlled": state.holder.is_some(),
        "agent_blocked": session.agent.is_blocked(),
        "redactions": state.redactions.len(),
        "redactions_full": state.redactions.saturated,
        "settings": state.settings,
        "network": state.network,
        "failed": state.failed,
    })
}

/// Where the administrator watches a session: the admin page, which opens that session's viewer once they are logged
/// in.
pub fn watch_url(app: &App, id: &str) -> String {
    let path = &app.config.static_path;
    let page = if path == "/" { "/".to_owned() } else { format!("{path}/") };
    format!("{}{page}#session={id}", app.config.origin)
}

/// The sessions `who` may reach (see [`App::reachable`]).
pub fn sessions_of(app: &App, who: app::Owner) -> Vec<Arc<Session>> {
    app.sessions.lock().unwrap().values().filter(|s| app.reachable(who, s)).cloned().collect()
}

/// A start URL and egress name as a caller gave them, checked: the URL http(s) and bounded, the egress one configured.
pub fn target<'a>(
    app: &'a App,
    url: &str,
    egress: Option<&str>,
) -> Result<(String, Option<&'a crate::egresses::Egress>), String> {
    let url = url.trim().to_owned();
    if url.len() > MAX_URL || !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!("the URL must be http(s) and at most {MAX_URL} bytes"));
    }
    let egress = match egress {
        None => None,
        Some(name) => match app.config.egresses.iter().find(|e| e.name == name) {
            Some(egress) => Some(egress),
            None => return Err("no egress has that name".into()),
        },
    };
    Ok((url, egress))
}

/// What a new session is started with.
pub struct Start<'a> {
    /// The new session's id (see [`session_id`]).
    pub id: String,
    pub url: String,
    pub devtools: bool,
    pub egress: Option<&'a crate::egresses::Egress>,
    pub kiosk: bool,
    pub lane: Option<u8>,
    pub owner: app::Owner,
    pub features: app::Features,
}

/// A new session id: 12 random hex digits.
pub fn session_id() -> String {
    (0..12).map(|_| char::from_digit(rand::random::<u32>() % 16, 16).expect("a hex digit")).collect()
}

/// Starts a session and lists it once its display is up; a session that fails to start, or whose display never comes
/// up, is undone.
pub async fn start(app: &Arc<App>, start: Start<'_>) -> Result<Arc<Session>, (StatusCode, String)> {
    let Start { id, url, devtools, egress, kiosk, lane, owner, features } = start;
    let Some(_reservation) = Reservation::take(app) else {
        return Err((StatusCode::CONFLICT, "too many sessions; stop one first".into()));
    };
    let dir = app.config.streams_dir.join(&id);
    if let Err(e) = std::fs::create_dir(&dir) {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
    }
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o770));
    let size = (app.config.width, app.config.height);
    // Through an egress, the browser says it is where that egress leaves, and the upstream's user name carries the
    // session's id for providers that keep one exit address per such session.
    let locale = egress.map_or(&app.config.locale, |e| &e.locale);
    let upstream = match egress.map(|e| e.upstream.for_session(&id)).transpose() {
        Ok(upstream) => upstream,
        Err(why) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("the egress: {why}")));
        }
    };
    let walled = app.config.demo.is_some();
    let new =
        NewSession { session: &id, url: &url, devtools, size, locale, lane, egress: upstream.as_ref(), kiosk, walled };
    let failed = match app.controller.create_session(&new).await {
        // The controller may still hold what it built: a rollback it could not complete, or a build it finishes after
        // this gave up waiting. It is stopped like any failed session.
        Err(e) => Some(e),
        Ok(()) => {
            let socket = dir.join("display.sock");
            let deadline = Instant::now() + DISPLAY_START;
            let mut ready = socket.exists();
            while !ready && Instant::now() < deadline {
                tokio::time::sleep(DISPLAY_POLL).await;
                ready = socket.exists();
            }
            // A display that never opened its stream socket cannot be watched.
            (!ready).then(|| {
                tracing::warn!(session = %id, timeout = ?DISPLAY_START, "the display did not open its stream socket");
                "the display did not start".to_owned()
            })
        }
    };
    let name = egress.map(|e| e.name.clone());
    let session = Arc::new(Session::new(id.clone(), url, devtools, name, owner, features, app.controller.clone()));
    if let Some(why) = failed {
        // Undone rather than listed. If the undo fails, the session stays listed as failed, so stopping it can be
        // retried.
        app.sessions.lock().unwrap().insert(id.clone(), session.clone());
        session.state.lock().unwrap().failed = Some(why.clone());
        return Err(match stop(app, &session).await {
            Ok(()) => (StatusCode::BAD_GATEWAY, why),
            Err(e) => (StatusCode::BAD_GATEWAY, format!("{why}, and stopping it failed: {e}")),
        });
    }
    // Listed only if its owner may still act: a token revoked while this started never gets it. The check and the
    // listing happen under the sessions lock, which the revocation's search for the token's sessions takes after it
    // marked the token revoked, so one of them always finds the session.
    let allowed = {
        let mut sessions = app.sessions.lock().unwrap();
        sessions.insert(id.clone(), session.clone());
        app.may_act(owner)
    };
    if !allowed {
        tracing::info!(session = %id, "its token was revoked while it started; stopping it");
        session.state.lock().unwrap().failed = Some("its token was revoked".into());
        let _ = stop(app, &session).await;
        return Err((StatusCode::UNAUTHORIZED, "the token was revoked".into()));
    }
    tracing::info!(session = %id, devtools, "session started");
    Ok(session)
}

pub async fn stop(app: &App, session: &Session) -> Result<(), String> {
    {
        let mut state = session.state.lock().unwrap();
        Session::revoke(&mut state);
        for viewer in state.viewers.values() {
            let _ = viewer.tx.try_send(app::ToViewer::Close("the session ended"));
        }
    }
    // A relay that cannot take the close (blocked on a viewer that stopped reading) ends anyway.
    session.ended.send_replace(true);
    // Its agent link gives its controller connection back first, so the stop itself finds one free.
    session.agent.detach();
    // A session the controller did not stop stays listed, marked failed, so stopping it can be retried.
    if let Err(e) = app.controller.stop_session(&session.id).await {
        session.state.lock().unwrap().failed = Some(format!("stopping failed: {e}"));
        return Err(e);
    }
    app.sessions.lock().unwrap().remove(&session.id);
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
    /// Too many calls wait for this session's agent already, or this one waited too long; it asked nothing.
    Busy,
    Failed(String),
}

/// One tool call on a session's agent for `who`, checked again once the agent admits it (see [`App::may_act`]). The
/// agent runs an admitted call as its own task (see `Agent::call`): a caller that disconnects cannot cut it.
pub async fn call(app: &App, who: app::Owner, session: &Session, tool: &str, arguments: Value) -> Called {
    if session.state.lock().unwrap().failed.is_some() {
        return Called::Stopped;
    }
    let arguments = if arguments.is_null() { json!({}) } else { arguments };
    let allowed = || app.may_act(who);
    match session.agent.call(tool, arguments, allowed).await {
        Ok(mut result) => {
            redactions(session).apply(&mut result);
            Called::Done(result)
        }
        Err(agent::CallError::Blocked) => Called::Controlled,
        Err(agent::CallError::Busy) => Called::Busy,
        Err(agent::CallError::Withdrawn) => Called::Revoked,
        Err(agent::CallError::Failed(why)) => Called::Failed(redacted_failure(&redactions(session), why)),
    }
}

/// The session's redaction contract, its matcher built (see [`crate::redact::Redactions::ready`]), to apply outside the
/// lock: a result may be tens of MiB, and every viewer's relay takes that lock for each message.
fn redactions(session: &Session) -> Scrub {
    let mut state = session.state.lock().unwrap();
    state.redactions.ready();
    state.redactions.scrub()
}

/// What the driver said went wrong, which may quote the page as well. Its error comes as JSON text, whose escaping could
/// hide a recorded string from the match: the JSON itself is redacted, before it is written out again.
fn redacted_failure(redactions: &Scrub, why: String) -> String {
    let mut why = serde_json::from_str::<Value>(&why).unwrap_or(Value::String(why));
    redactions.apply(&mut why);
    why.as_str().map_or_else(|| why.to_string(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_driver_error_loses_what_the_person_typed_even_when_escaped() {
        let mut redactions = crate::redact::Redactions::default();
        redactions.keep("pa\"ss\\word");
        let redactions = redactions.scrub();
        let error = json!({"code": -32000, "message": "no field named pa\"ss\\word"}).to_string();
        let shown = redacted_failure(&redactions, error);
        assert!(!shown.contains("word") && shown.contains("[redacted]"), "{shown}");
        assert_eq!(redacted_failure(&redactions, "agent: pa\"ss\\word".into()), "agent: [redacted]");
    }
}
