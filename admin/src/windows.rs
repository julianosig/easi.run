//! `close_window`: the agent closes the browser window shown, as that window's close button would. While Chrome has more
//! than one window (a page's popup, a site's app window), the display refuses the driver's keyboard: its tools name a
//! PID, which every window of the browser shares, so a key could reach another window than the one meant (see the
//! display's `inject.rs`). Closing the window shown names none, so nothing is ambiguous, and once one window is left the
//! keyboard works again. Like the desktop tools, it waits for no one and does nothing while a person holds control.

use std::sync::LazyLock;

use serde_json::{Value, json};

use crate::agent::CallError;
use crate::app::{App, Owner, Session};
use crate::service::Called;

pub const NAME: &str = "close_window";

/// What a driver's refusal says when the keyboard was refused for several windows, or for no window known in front.
const REFUSALS: [&str; 3] = ["ambiguous-root-pid", "ambiguous-pid", "no verified foreground target"];

/// What the agent is told beside such a refusal.
pub const HINT: &str = "If the browser has more than one window open (a popup, an app window), its keyboard is refused \
    until one is left: close_window closes the window shown on the screen, as its close button would; read the screen \
    again after it.";

pub fn known(name: &str) -> bool {
    name == NAME
}

static TOOLS: LazyLock<Vec<Value>> = LazyLock::new(|| {
    vec![json!({
        "name": NAME,
        "description": "Close the browser window shown on the screen (what get_desktop_state captures), as its close \
            button would; a page may ask to stay. The window under it then shows; closing the last one opens the browser \
            again at the session's first page. While the browser has more than one window (a popup, a site's app \
            window), the keyboard tools are refused: close the windows you do not need first.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
        "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": false},
    })]
});

pub fn tools() -> &'static [Value] {
    &TOOLS
}

/// Whether a driver's result is a keyboard refused for the browser's windows (see [`HINT`]): an error, which a page's
/// text quoted in a successful result never is.
pub fn refused_for_windows(result: &Value) -> bool {
    if result["isError"] != true {
        return false;
    }
    let text = result["content"].as_array().into_iter().flatten().filter_map(|item| item["text"].as_str());
    text.into_iter().any(|text| REFUSALS.iter().any(|refusal| text.contains(refusal)))
}

pub async fn call(app: &App, who: Owner, session: &Session, arguments: Value) -> Called {
    if app.config.demo.is_some() {
        return Called::Failed("not in the demo".into());
    }
    if !app.may_act(who) {
        return Called::Revoked;
    }
    if !app.reachable(who, session) {
        return Called::Unreached;
    }
    if arguments.as_object().is_none_or(|arguments| !arguments.is_empty()) {
        return Called::Failed(format!("{NAME} takes no arguments"));
    }
    // A place among the session's calls, and one that counts as using the session; the agent's turn comes after it.
    let Some(_turn) = session.agent.place() else { return Called::Busy };
    session.agent.used_now();
    match session.agent.close_window(|| app.may_act(who) && app.reachable(who, session)).await {
        Ok(answer) if answer == "ok" => Called::Done(json!({
            "content": [{"type": "text", "text": "Asked the window shown to close. Read the screen again: the window \
                under it shows now, or, if it was the last, the browser opens again at the session's first page."}],
        })),
        Ok(answer) if answer == "err control-held" => Called::Controlled,
        Ok(answer) if answer == "err no-window" => Called::Failed("the browser shows no window now".into()),
        Ok(answer) => Called::Failed(format!("the display refused: {answer}")),
        Err(CallError::Blocked) => Called::Controlled,
        Err(CallError::Busy) => Called::Busy,
        Err(CallError::Withdrawn) if app.may_act(who) => Called::Unreached,
        Err(CallError::Withdrawn) => Called::Revoked,
        Err(CallError::Failed(why)) => Called::Failed(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keyboard_refused_for_several_windows_is_told_apart() {
        let refused = |text: &str| json!({"content": [{"type": "text", "text": text}], "isError": true});
        assert!(refused_for_windows(&refused("cua-compositor rejected the command: ambiguous-root-pid")));
        assert!(refused_for_windows(&refused(
            "foreground_unavailable: cua-compositor has no verified foreground target; call bring_to_front"
        )));
        assert!(!refused_for_windows(&refused("cua-compositor rejected the command: control-held")));
        assert!(!refused_for_windows(&json!({"content": "not a list"})));
        // A page that says it, read successfully, is not a refusal.
        assert!(!refused_for_windows(&json!({"content": [{"type": "text", "text": "ambiguous-pid"}]})));
    }
}
