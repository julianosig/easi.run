//! The agent and the person: an agent that needs a person (a login, a challenge, something to confirm) asks for one with
//! `request_person`, and `wait_for_control` waits until it may act again. Both are session tools, beside the driver's
//! and the files' (see `service::call`), over the agent API and MCP alike; neither runs on the agent's link.
//!
//! A request is the session's `help` (see `app::State`): the administrator's page shows it, with what the agent wrote,
//! the timeline records it, and the webhook is told (see `notify`). It stays open until a person who held control gives
//! it back or goes away, the administrator dismisses it, or the agent asks again (which replaces it); stopping the
//! session ends it. While it is open, or a person holds control, `wait_for_control` waits, up to [`MAX_WAIT`] a call;
//! it answers who has control then and whether a person held it since the agent was last told, as the agent's next
//! tool result also says once (see `service::PERSON_ACTED`).

use std::sync::LazyLock;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::{App, Help, Owner, Session};
use crate::service::Called;

/// What an agent may ask a person for.
pub const REASONS: [&str; 4] = ["login", "captcha", "confirm", "other"];
/// The longest message a request carries, in characters.
pub const MAX_MESSAGE: usize = 300;
/// The longest a `wait_for_control` call waits, and the default.
pub const MAX_WAIT: Duration = Duration::from_secs(50);
/// How often a wait looks whether the agent may act again once a person gave control back (its restart, if a takeover
/// stopped it, ends then).
const SETTLE: Duration = Duration::from_millis(100);

const NAMES: [&str; 2] = ["request_person", "wait_for_control"];

pub fn known(name: &str) -> bool {
    NAMES.contains(&name)
}

static TOOLS: LazyLock<Vec<Value>> = LazyLock::new(|| {
    vec![
        json!({
            "name": "request_person",
            "description": format!("Ask the person for help when only they can go on: `login` (a sign-in, a one-time \
                code, a passkey), `captcha` (a challenge for humans), `confirm` (a choice or payment that is theirs to \
                make) or `other`. `message` says what you need, at most {MAX_MESSAGE} characters, in the person's \
                words (never a password or anything secret). The session's page shows it, its timeline records it, \
                and the administrator's webhook, if set, is told. It returns the watch_url to give the person. Then \
                tell the person what you need and where, and call wait_for_control, which returns once they have given \
                control back. Asking again replaces the open request."),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "reason": {"type": "string", "enum": REASONS},
                    "message": {"type": "string", "maxLength": MAX_MESSAGE},
                },
                "required": ["reason"],
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
        }),
        json!({
            "name": "wait_for_control",
            "description": format!("Wait until you may act on this session again: while a person holds control, or \
                your request_person is open, this waits up to `timeout_s` seconds (default and most {}), then answers \
                `control` (`agent`: act again; `person`: they still hold it; `waiting`: your request is open and \
                nobody has taken control yet) and `person_acted` (a person held control since you were last told: read \
                the window again before acting). Call it again to keep waiting.", MAX_WAIT.as_secs()),
            "inputSchema": {
                "type": "object",
                "properties": {"timeout_s": {"type": "integer", "minimum": 1, "maximum": MAX_WAIT.as_secs()}},
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
        }),
    ]
});

pub fn tools() -> &'static [Value] {
    &TOOLS
}

fn result(value: Value) -> Called {
    Called::Done(
        json!({"content": [{"type": "text", "text": value.to_string()}], "structuredContent": value, "isError": false}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    reason: String,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wait {
    timeout_s: Option<u64>,
}

pub async fn call(app: &App, who: Owner, session: &Session, tool: &str, arguments: Value) -> Called {
    if !app.may_act(who) {
        return Called::Revoked;
    }
    match tool {
        "request_person" => request(app, who, session, arguments),
        _ => wait(app, who, session, arguments).await,
    }
}

fn request(app: &App, who: Owner, session: &Session, arguments: Value) -> Called {
    let usage = format!(
        "request_person takes {{reason: one of {}, message?: at most {MAX_MESSAGE} characters}}",
        REASONS.join(", ")
    );
    let Ok(asked) = serde_json::from_value::<Request>(arguments) else { return Called::Failed(usage) };
    let Some(reason) = REASONS.iter().copied().find(|r| *r == asked.reason) else { return Called::Failed(usage) };
    let message = asked.message.trim().to_owned();
    if message.chars().count() > MAX_MESSAGE {
        return Called::Failed(usage);
    }
    {
        let mut state = session.state.lock().unwrap();
        session.close_help(&mut state, "withdrawn");
        let since = crate::timeline::now_ms();
        state.help = Some(Help { reason, message: message.clone(), since });
        crate::app::list_changed();
        crate::app::Session::announce(&state);
        let event = json!({"state": "asked", "reason": reason, "message": message});
        session.agent.timeline.push("help", crate::service::by(event, &crate::service::caller(app, who)));
    }
    let watch_url = crate::service::watch_url(app, &session.id);
    let label = crate::service::label(session);
    let about = crate::notify::About { id: &session.id, label: &label, watch_url: &watch_url };
    app.notify.send(crate::notify::Event::PersonNeeded { reason, message: &message }, Some(about));
    result(json!({
        "requested": true,
        "watch_url": watch_url,
        "next": "Tell the person what you need and give them the watch_url; then call wait_for_control.",
    }))
}

async fn wait(app: &App, who: Owner, session: &Session, arguments: Value) -> Called {
    let usage = format!("wait_for_control takes {{timeout_s?: 1 to {}}}", MAX_WAIT.as_secs());
    let Ok(Wait { timeout_s }) = serde_json::from_value::<Wait>(arguments) else { return Called::Failed(usage) };
    let limit = match timeout_s {
        None => MAX_WAIT,
        Some(s @ 1..) if s <= MAX_WAIT.as_secs() => Duration::from_secs(s),
        Some(_) => return Called::Failed(usage),
    };
    let mut waits = session.state.lock().unwrap().waits.subscribe();
    let mut ended = session.ended.subscribe();
    let free = async {
        loop {
            if waits.wait_for(|w| !*w).await.is_err() {
                return;
            }
            // Given back: the agent acts again once it is no longer held (restarted, if a takeover stopped it).
            if !session.agent.is_blocked() {
                return;
            }
            tokio::time::sleep(SETTLE).await;
        }
    };
    tokio::select! {
        () = free => {}
        () = tokio::time::sleep(limit) => {}
        _ = ended.wait_for(|e| *e) => return Called::Stopped,
    }
    // Its caller may have lost the session meanwhile: a token revoked, or no longer allowed its saved session.
    if !app.may_act(who) {
        return Called::Revoked;
    }
    if !app.reachable(who, session) {
        return Called::Unreached;
    }
    let mut state = session.state.lock().unwrap();
    let control = match (state.holder.is_some(), state.help.is_some()) {
        (true, _) => "person",
        (false, true) => "waiting",
        (false, false) if session.agent.is_blocked() => "person",
        (false, false) => "agent",
    };
    let person_acted = control == "agent" && std::mem::take(&mut state.person_acted);
    let mut answer = json!({"control": control, "person_acted": person_acted});
    if person_acted {
        answer["note"] = json!(crate::service::PERSON_ACTED);
    }
    if let Some(help) = &state.help {
        answer["request"] = json!({"reason": help.reason, "message": help.message, "since": help.since});
    }
    result(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Features;

    fn text(called: &Called) -> Value {
        match called {
            Called::Done(result) => result["structuredContent"].clone(),
            _ => panic!("not done"),
        }
    }

    #[tokio::test]
    async fn a_request_stays_open_until_a_person_gives_control_back_and_the_wait_then_says_they_acted() {
        let app = crate::app::test_app(false);
        let session = std::sync::Arc::new(Session::test("s1", Owner::Local, Features::default()));
        let asked =
            call(&app, Owner::Local, &session, "request_person", json!({"reason": "login", "message": "Sign in"}));
        assert_eq!(text(&asked.await)["requested"], true);
        assert_eq!(session.state.lock().unwrap().help.as_ref().map(|h| h.reason), Some("login"));
        // Open, nobody holding control: the wait answers at its bound that the request is open.
        let waited = call(&app, Owner::Local, &session, "wait_for_control", json!({"timeout_s": 1})).await;
        assert_eq!(text(&waited)["control"], "waiting");
        assert_eq!(text(&waited)["request"]["message"], "Sign in");
        // A person takes control, then gives it back: the wait returns at once, saying they acted, once.
        let waiting = {
            let (app, session) = (app.clone(), session.clone());
            tokio::spawn(async move { call(&app, Owner::Local, &session, "wait_for_control", json!({})).await })
        };
        {
            let mut state = session.state.lock().unwrap();
            Session::set_holder(&mut state, Some(7));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished(), "waits while a person holds control");
        {
            let mut state = session.state.lock().unwrap();
            Session::give_back(&mut state);
            session.close_help(&mut state, "done");
        }
        let answer = text(&tokio::time::timeout(Duration::from_secs(5), waiting).await.unwrap().unwrap());
        assert_eq!((&answer["control"], &answer["person_acted"]), (&json!("agent"), &json!(true)));
        assert!(answer.get("request").is_none());
        let again = call(&app, Owner::Local, &session, "wait_for_control", json!({"timeout_s": 1})).await;
        assert_eq!(text(&again)["person_acted"], false, "told once");
        let events = session.agent.timeline.page(0)["events"].clone();
        let help: Vec<&Value> = events.as_array().unwrap().iter().filter(|e| e["kind"] == "help").collect();
        assert_eq!(help.iter().map(|e| e["state"].as_str().unwrap()).collect::<Vec<_>>(), ["asked", "done"]);
        assert_eq!(help[0]["via"], "local");
    }

    #[tokio::test]
    async fn a_request_names_a_known_reason_and_a_bounded_message() {
        let app = crate::app::test_app(false);
        let session = Session::test("s1", Owner::Local, Features::default());
        for bad in [
            json!({"reason": "money"}),
            json!({"reason": "login", "message": "x".repeat(MAX_MESSAGE + 1)}),
            json!({"reason": "login", "extra": 1}),
        ] {
            assert!(matches!(call(&app, Owner::Local, &session, "request_person", bad).await, Called::Failed(_)));
        }
        assert!(matches!(
            call(&app, Owner::Local, &session, "wait_for_control", json!({"timeout_s": 51})).await,
            Called::Failed(_)
        ));
        assert!(session.state.lock().unwrap().help.is_none());
    }
}
