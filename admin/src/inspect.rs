//! `inspect_page`: what an agent reads of a page's structure in any session, with no debugging port. The display carries
//! the question to the browser's launcher, which reads the page over its private DevTools pipe, in a world of its own the
//! page cannot see (`browser/launch/src/inspect.js`). It reads, and runs nothing an agent wrote: a CSS selector is all an
//! agent gives. A password field's value is never read, and what a person typed is taken out of the answer as out of
//! any tool's (see `redact`). Like the desktop tools, it waits for no one and answers nothing while a person holds
//! control.
//!
//! `run_javascript` is what a person may add by starting a session with Developer tools: the driver's own script tool,
//! over Chrome's debugging endpoint, in the page's own world. It may read a password field, and the page may notice:
//! the person chose that for this session, and no other has it.

use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use display_protocol::Direction;
use display_protocol::frame::{FrameReader, Link, MAX_CHUNK, MAX_INSPECT_QUERY};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::app::{App, Owner, Session};
use crate::service::Called;

pub const NAME: &str = "inspect_page";
/// The most an agent gets of an element's text, and of an attribute's value, in characters.
const MAX_TEXT: usize = 2000;
const MAX_ATTRIBUTE: usize = 500;
const SCRIPT: &str = "run_javascript";
/// The driver's tool `run_javascript` runs as.
pub const SCRIPT_TOOL: &str = "page";
/// The elements an answer holds by default, and at most.
const DEFAULT_ELEMENTS: u64 = 20;
const MAX_ELEMENTS: u64 = 200;
/// The longest selector taken.
const MAX_SELECTOR: usize = 1024;
/// How long the page may take to answer: a little past what the display gives the launcher.
const ANSWER_DEADLINE: Duration = Duration::from_secs(10);

pub fn known(name: &str) -> bool {
    name == NAME
}

pub fn is_script(name: &str) -> bool {
    name == SCRIPT
}

/// Whether an agent may name this tool: both here.
pub fn listed(name: &str) -> bool {
    known(name) || is_script(name)
}

static TOOLS: LazyLock<Vec<Value>> = LazyLock::new(|| {
    vec![
        json!({
            "name": NAME,
            "description": "Read the structure of the page in the active tab: the elements a CSS selector matches, each with \
                its tag, attributes, text and whether it shows, and with `html` its HTML. Use it for what the window's tree \
                does not say: links, ids, classes, data attributes, hidden elements. It changes nothing on the page, and \
                the page cannot see it. Never includes a password field's value.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": {"type": "string", "maxLength": MAX_SELECTOR, "description": "A CSS selector."},
                    "html": {"type": "boolean", "description": "Include each element's HTML. Default false."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_ELEMENTS,
                        "description": format!("The most elements to return. Default {DEFAULT_ELEMENTS}.")},
                },
                "required": ["selector"],
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
        }),
        json!({
            "name": SCRIPT,
            "description": "Run JavaScript in the page's own world and return its result, for what inspect_page cannot say: \
                state in the page's variables, or changing the page. Only in a session the person started with Developer \
                tools; elsewhere it is refused, and inspect_page still reads a page's structure.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "javascript": {"type": "string", "description": "The expression to evaluate."},
                    "pid": {"type": "integer", "description": "The browser's pid, from list_windows."},
                    "window_id": {"type": "integer", "description": "Its window, from list_windows."},
                },
                "required": ["javascript", "pid", "window_id"],
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true},
        }),
    ]
});

pub fn tools() -> &'static [Value] {
    &TOOLS
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Script {
    javascript: String,
    pid: u64,
    window_id: u64,
}

/// The driver's arguments for a `run_javascript`, in a session with Developer tools or none.
pub fn script(devtools: bool, arguments: Value) -> Result<Value, String> {
    if !devtools {
        return Err("this session runs without Developer tools, which the person chooses as they start or resume it: \
            inspect_page reads a page's structure in any session"
            .into());
    }
    let script: Script = serde_json::from_value(arguments)
        .map_err(|_| format!("{SCRIPT} takes {{javascript, pid, window_id}}, the last two from list_windows"))?;
    Ok(
        json!({"action": "execute_javascript", "javascript": script.javascript, "pid": script.pid, "window_id": script.window_id}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asked {
    selector: String,
    #[serde(default)]
    html: bool,
    limit: Option<u64>,
}

fn usage() -> String {
    format!(
        "{NAME} takes {{selector: a CSS selector of at most {MAX_SELECTOR} bytes, html?: true, limit?: 1 to {MAX_ELEMENTS}}}"
    )
}

/// The question as the launcher reads it, from what an agent sent.
fn question(arguments: Value, held: usize) -> Result<String, String> {
    let asked: Asked = serde_json::from_value(arguments).map_err(|_| usage())?;
    let limit = asked.limit.unwrap_or(DEFAULT_ELEMENTS);
    if asked.selector.trim().is_empty() || asked.selector.len() > MAX_SELECTOR || !(1..=MAX_ELEMENTS).contains(&limit) {
        return Err(usage());
    }
    // Past where it is cut, by as much as a person's typing could be: cut any shorter, what they typed would no longer be
    // recognized (see `trimmed`), and its start would be left to read.
    let query = json!({
        "selector": asked.selector,
        "html": asked.html,
        "limit": limit,
        "text": MAX_TEXT + held,
        "attribute": MAX_ATTRIBUTE + held,
    })
    .to_string();
    // Its escapes can only grow it so far: past what the link takes, it is refused as too long.
    if query.len() > MAX_INSPECT_QUERY { Err(usage()) } else { Ok(query) }
}

/// Asks the display whose stream socket is `path`, on a link of its own, and returns what the page answered.
async fn ask(path: &Path, query: String) -> Result<String, String> {
    let exchange = async {
        let mut stream = UnixStream::connect(path).await.map_err(|e| format!("display: {e}"))?;
        stream.write_all(&Link::Inspect { query }.encode()).await.map_err(|e| format!("display: {e}"))?;
        let mut frames = FrameReader::new(Direction::ServerToClient);
        let mut chunk = vec![0u8; MAX_CHUNK];
        loop {
            let n = stream.read(&mut chunk).await.map_err(|e| format!("display: {e}"))?;
            if n == 0 {
                return Err("the display closed its link before it answered".to_owned());
            }
            frames.push(&chunk[..n]).map_err(|e| format!("display: {e}"))?;
            while let Some(body) = frames.next_body().map_err(|e| format!("display: {e}"))? {
                if let Ok(Link::Inspected { answer }) = Link::decode(&body, Direction::ServerToClient) {
                    return Ok(answer);
                }
            }
        }
    };
    tokio::time::timeout(ANSWER_DEADLINE, exchange)
        .await
        .unwrap_or_else(|_| Err("the page did not answer in time".into()))
}

/// What an agent gets of what the page answered. An empty answer is the browser's: no page, or too much of one.
fn answered(answer: &str, held: usize) -> Result<Value, String> {
    if answer.is_empty() {
        return Err("the page did not answer: try a narrower selector, or a smaller limit".into());
    }
    let mut value: Value = serde_json::from_str(answer).map_err(|_| "the page's answer was not readable".to_owned())?;
    if let Some(why) = value["error"].as_str() {
        return Err(why.to_owned());
    }
    if let Some(fields) = value.as_object_mut() {
        fields.remove("visible");
        fields.remove("focused");
    }
    // HTML escapes what a person typed out of what the redaction looks for, so it reads it as they typed it.
    if held > 0
        && let Some(elements) = value["elements"].as_array_mut()
    {
        for element in elements {
            if let Some(html) = element["html"].as_str() {
                element["html"] = Value::String(unescaped(html));
            }
        }
    }
    Ok(value)
}

/// `html` without the escapes a browser writes in it.
fn unescaped(html: &str) -> String {
    let named = html.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&nbsp;", "\u{a0}");
    named.replace("&amp;", "&")
}

/// What an agent gets of a redacted answer: each element's text and attributes cut to their bounds, only now that what a
/// person typed is out of them.
pub fn trimmed(mut result: Value) -> Value {
    let Some(page) = result.get_mut("structuredContent") else { return result };
    if let Some(elements) = page.get_mut("elements").and_then(Value::as_array_mut) {
        for element in elements {
            if let Some(text) = element.get("text").and_then(Value::as_str) {
                element["text"] = Value::String(text.trim().to_owned());
            }
            cut(element.get_mut("text"), MAX_TEXT);
            if let Some(attributes) = element.get_mut("attributes").and_then(Value::as_object_mut) {
                attributes.values_mut().for_each(|value| cut(Some(value), MAX_ATTRIBUTE));
            }
        }
    }
    let text = page.to_string();
    if let Some(first) = result.pointer_mut("/content/0/text") {
        *first = Value::String(text);
    }
    result
}

fn cut(value: Option<&mut Value>, most: usize) {
    if let Some(value) = value
        && let Some(text) = value.as_str()
        && text.chars().count() > most
    {
        *value = Value::String(text.chars().take(most).collect());
    }
}

fn result(value: Value) -> Value {
    json!({"content": [{"type": "text", "text": value.to_string()}], "structuredContent": value, "isError": false})
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
    // One call of an agent's at a time, and one that counts as using the session.
    let Some(_turn) = session.agent.place() else { return Called::Busy };
    session.agent.used_now();
    let _alone = session.inspecting.lock().await;
    if session.agent.is_blocked() {
        return Called::Controlled;
    }
    let held = session.state.lock().unwrap().redactions.held();
    let query = match question(arguments, held) {
        Ok(query) => query,
        Err(why) => return Called::Failed(why),
    };
    let path = app.config.streams_dir.join(&session.id).join("display.sock");
    let answer = ask(&path, query).await;
    // A person who took control meanwhile: what the page showed then is theirs.
    if session.agent.is_blocked() {
        return Called::Controlled;
    }
    match answer.and_then(|answer| answered(&answer, held)) {
        Ok(value) => Called::Done(result(value)),
        Err(why) => Called::Failed(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_takes_a_selector_and_stays_within_its_bounds() {
        let asked = question(json!({"selector": "a[href]", "html": true, "limit": 5}), 7).unwrap();
        let asked: Value = serde_json::from_str(&asked).unwrap();
        let (text, attribute) = (MAX_TEXT + 7, MAX_ATTRIBUTE + 7);
        let expected = json!({"selector": "a[href]", "html": true, "limit": 5, "text": text, "attribute": attribute});
        assert_eq!(asked, expected);
        let default: Value = serde_json::from_str(&question(json!({"selector": "a"}), 0).unwrap()).unwrap();
        assert_eq!(default["limit"], DEFAULT_ELEMENTS);
        for bad in [
            json!({}),
            json!({"selector": "  "}),
            json!({"selector": "a", "limit": 0}),
            json!({"selector": "a", "limit": MAX_ELEMENTS + 1}),
            json!({"selector": "a".repeat(MAX_SELECTOR + 1)}),
            json!({"selector": "a", "script": "x"}),
        ] {
            assert!(question(bad.clone(), 0).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_script_runs_only_in_a_session_with_developer_tools() {
        let asked = json!({"javascript": "1+1", "pid": 3, "window_id": 4});
        assert!(script(false, asked.clone()).unwrap_err().contains("Developer tools"));
        let driver = script(true, asked).unwrap();
        assert_eq!(driver, json!({"action": "execute_javascript", "javascript": "1+1", "pid": 3, "window_id": 4}));
        assert!(script(true, json!({"javascript": "1"})).is_err());
        assert!(script(true, json!({"javascript": "1", "pid": 3, "window_id": 4, "action": "click_element"})).is_err());
    }

    #[test]
    fn an_answer_says_the_page_and_a_failure_says_why() {
        let page = answered(r#"{"visible":true,"url":"https://a.test/","count":0,"elements":[]}"#, 0).unwrap();
        assert_eq!(page["url"], "https://a.test/");
        assert!(page.get("visible").is_none());
        assert_eq!(answered(r#"{"error":"not a valid CSS selector"}"#, 0).unwrap_err(), "not a valid CSS selector");
        assert!(answered("", 0).is_err());
        assert!(answered("not json", 0).is_err());
    }

    #[test]
    fn html_reads_as_typed_once_there_is_typing_to_recognize() {
        let html = r#"<p title="A &amp;amp; &quot;B&quot;">C &amp; D &lt;e&gt;</p>"#;
        let answer = json!({"elements": [{"html": html}]}).to_string();
        assert_eq!(answered(&answer, 4).unwrap()["elements"][0]["html"], r#"<p title="A &amp; "B"">C & D <e></p>"#);
        assert_eq!(answered(&answer, 0).unwrap()["elements"][0]["html"], html);
    }

    #[test]
    fn an_element_is_cut_to_its_bounds_after_the_redaction_and_both_forms_say_the_same() {
        let long = format!(" \n{}", "é".repeat(MAX_TEXT + 50));
        let page = json!({"elements": [{"text": long, "attributes": {"a": "x".repeat(MAX_ATTRIBUTE + 1), "b": "y"}}]});
        let note = json!({"type": "text", "text": "a note"});
        let answer = json!({"content": [{"type": "text", "text": page.to_string()}, note], "structuredContent": page});
        let cut = trimmed(answer);
        let element = &cut["structuredContent"]["elements"][0];
        assert!(element["text"].as_str().unwrap().starts_with('é'), "trimmed too");
        assert_eq!(element["text"].as_str().unwrap().chars().count(), MAX_TEXT);
        assert_eq!(element["attributes"]["a"].as_str().unwrap().len(), MAX_ATTRIBUTE);
        assert_eq!(element["attributes"]["b"], "y");
        assert_eq!(cut["content"][0]["text"], cut["structuredContent"].to_string());
        assert_eq!(cut["content"][1]["text"], "a note");
        // What the set being full left of an answer has nothing to cut.
        let hidden = json!({"content": [{"type": "text", "text": "hidden"}], "isError": true});
        assert_eq!(trimmed(hidden.clone()), hidden);
    }
}
