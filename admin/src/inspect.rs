//! `inspect_page`: what an agent reads of a page's structure in any session, with no debugging port. The display carries
//! the question to the browser's launcher, which reads the page over its private DevTools pipe, in a world of its own the
//! page cannot see (`browser/launch/src/inspect.js`). It reads, and runs nothing an agent wrote: a CSS selector is all an
//! agent gives. A password field's value is never read, and what a person typed is taken out of the answer as out of
//! any tool's (see `redact`). It runs beside the desktop tools' calls, after any other question of the page under way
//! (a reading, a script, a navigation: `Session::inspecting`), and answers nothing while a person holds control.
//!
//! `run_javascript` is what a person may add by starting a session with Developer tools: a script the agent wrote,
//! run on the page shown, in the page's own world, as a person's gesture. It goes the way a question about the page
//! goes, to the browser's launcher, which evaluates it over its private DevTools pipe and in a session with Developer
//! tools alone (the launcher's own knowledge, not the admin's word). It may read a password field, and the page may
//! notice: the person chose that for this session, and no other has it. Its answer reads as the driver's script tool
//! gave it while that tool existed (`cdp.runtime.evaluate.user_gesture: ...`), which agents already read.

use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use crate::display_link::DisplayLink;
use display_protocol::frame::{
    Link, MAX_INSPECT_QUERY, MAX_NAVIGATE, MAX_SCRIPT, MAX_SCRIPT_PART, NAVIGATE_WAIT, SCRIPT_WAIT,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::{App, Owner, Session};
use crate::service::Called;

pub const NAME: &str = "inspect_page";
/// The most an agent gets of an element's text, and of an attribute's value, in characters.
const MAX_TEXT: usize = 2000;
const MAX_ATTRIBUTE: usize = 500;
const SCRIPT: &str = "run_javascript";
const NAVIGATE: &str = "navigate";
/// What a script's answer starts with: the label the driver's script tool gave it.
const SCRIPT_LABEL: &str = "cdp.runtime.evaluate.user_gesture";
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

pub fn is_navigate(name: &str) -> bool {
    name == NAVIGATE
}

/// Whether an agent may name this tool: all three here.
pub fn listed(name: &str) -> bool {
    known(name) || is_script(name) || is_navigate(name)
}

/// What a page read takes, for `inspect_page` and for `navigate`'s `read`.
fn read_properties() -> Value {
    json!({
        "selector": {"type": "string", "maxLength": MAX_SELECTOR, "description": "A CSS selector, matched in the \
            page's own document: not inside its frames or shadow roots, so none found does not mean none shows."},
        "html": {"type": "boolean", "description": "Include each element's HTML, to look at, not to parse: once the session \
            holds text a person typed or pasted that is kept out of what you read, `&lt;` `&gt;` `&quot;` `&nbsp;` and `&amp;` in it come back decoded, \
            once (a quote inside an attribute among them), so take attribute values from `attributes`. Default false."},
        "limit": {"type": "integer", "minimum": 1, "maximum": MAX_ELEMENTS,
            "description": format!("The most elements to return. Default {DEFAULT_ELEMENTS}. `count` is how many \
                match; `elements`, the first that fit within this and about 200 kB; `truncated`, that some were left \
                out: narrow the selector before taking what came for all there is. Each element's text is cut at \
                {MAX_TEXT} characters and each attribute's value at {MAX_ATTRIBUTE}, which `truncated` does not say: \
                a value that long may be cut (a link's address among them).")},
    })
}

static TOOLS: LazyLock<Vec<Value>> = LazyLock::new(|| {
    vec![
        json!({
            "name": NAME,
            "description": "Read the structure of the page in the active tab: the elements a CSS selector matches, each with \
                its tag, attributes, text and `shown` (whether it has a box on the page, as checkVisibility() says by \
                default: one hidden by `visibility` may still say true, so see the window's tree before clicking), \
                and with `html` its HTML. Use it for what the window's tree does not say: links, ids, classes, data \
                attributes, hidden elements. It changes nothing on the page, and the page cannot see it. Never includes a \
                password field's value. The tab read is the one that shows (the one with the keyboard when several \
                windows show one, else the first). When none says it shows: if a tab has a dialog open, nothing is \
                read and the error says so; otherwise the first tab that answers is read, which `url` and `title` \
                name.",
            "inputSchema": {
                "type": "object",
                "properties": read_properties(),
                "required": ["selector"],
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
        }),
        json!({
            "name": NAVIGATE,
            "description": "Go to an address in the tab that shows (the one with the keyboard, or the only one that \
                shows; when that is in doubt, no tab is taken anywhere and the error says so), and answer once the page \
                has loaded: what became of the \
                navigation (`outcome`: loaded, same_document or download; or, as an error, failed, timeout, dialog, closed, \
                crashed or stopped, with `began` saying whether the browser had begun loading a new document, so \
                the page may have left the one it was on: read it before relying on either), and, when \
                the page loaded and answered, the address, title and HTTP status of the page it is on. With `read`, also read the page there, as \
                inspect_page reads it, in the same call: a page that did not load is not read. An outcome of loaded \
                or same_document means the page was reached, even when the answer is an error because the page \
                could not be read as asked or did not say where it is (`read_error` says why): read it, do not \
                navigate again for that.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": {"type": "string", "maxLength": display_protocol::session::MAX_URL,
                        "description": "An http(s) address, or about:blank."},
                    "read": {
                        "type": "object",
                        "description": "What to read of the page once it loaded, as inspect_page takes it.",
                        "properties": read_properties(),
                        "required": ["selector"],
                        "additionalProperties": false,
                    },
                },
                "required": ["url"],
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": true},
        }),
        json!({
            "name": SCRIPT,
            "description": "Run JavaScript in the page's own world and return its result, for what inspect_page cannot say: \
                state in the page's variables, or changing the page. Only in a session the person started with Developer \
                tools; elsewhere it is refused, and inspect_page still reads a page's structure. It runs in a tab that \
                shows: the one with the keyboard, else the first that answers it shows (a tab whose dialog is open is \
                not asked); `pid` and `window_id` are asked as the driver's own tool asks them, and choose nothing. \
                The answer, after `cdp.runtime.evaluate.user_gesture: `, is the value as JSON when it is one; an \
                object that is none (an element, say) comes as what it is called, so return `JSON.stringify(...)` of \
                what you need. A script that throws answers `exception: ` and the browser's description of it, not an error. \
                An answer past 1 MiB, or a script past 30 seconds, gives no answer at all though what it changed \
                stays: read the page before running a script that changes it again. So too after it was ended by a \
                dialog of the page: the browser may still run it once the dialog is answered.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "javascript": {"type": "string", "description": "The expression to evaluate."},
                    "pid": {"type": "integer", "description": "The browser's pid, from list_windows (it chooses no tab)."},
                    "window_id": {"type": "integer", "description": "Its window, from list_windows (it chooses no tab)."},
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

/// The script of a `run_javascript`, in a session with Developer tools or none. `pid` and `window_id` are taken as
/// they always were, and the script runs on the page shown.
pub fn script(devtools: bool, arguments: Value) -> Result<String, String> {
    if !devtools {
        return Err("this session runs without Developer tools, which the person chooses as they start or resume it: \
            inspect_page reads a page's structure in any session"
            .into());
    }
    let script: Script = serde_json::from_value(arguments).map_err(|_| {
        format!("{SCRIPT} takes {{javascript, pid, window_id}}: pid and window_id, from list_windows, are required and ignored")
    })?;
    let _ = (script.pid, script.window_id);
    if script.javascript.len() > MAX_SCRIPT {
        return Err(format!("{SCRIPT} takes a script of at most {MAX_SCRIPT} bytes"));
    }
    Ok(script.javascript)
}

/// What the browser answered a script's evaluation (`{"result": ...}` as `Runtime.evaluate` answered), as the
/// driver's script tool said it: an exception's description, the value as JSON, or what the value is called.
fn evaluated(response: &Value) -> String {
    if let Some(exception) = response["result"].get("exceptionDetails") {
        let said = exception["exception"]["description"].as_str().map(str::to_owned);
        return format!("exception: {}", said.unwrap_or_else(|| exception.to_string()));
    }
    if let Some(result) = response["result"].get("result") {
        if let Some(value) = result.get("value") {
            return value.to_string();
        }
        if let Some(described) = result["description"].as_str() {
            return described.to_owned();
        }
    }
    response.to_string()
}

/// A tool result that says `text`, failed or not, as the driver's tools say theirs.
fn said(text: String, failed: bool) -> Value {
    let mut result = json!({"content": [{"type": "text", "text": text}]});
    if failed {
        result["isError"] = json!(true);
    }
    result
}

/// What an agent gets of a script's answer from the display: the value it gave, or why there is none.
fn scripted(answer: Result<String, String>) -> Value {
    let answer = match answer {
        Ok(answer) => answer,
        Err(why) => return said(why, true),
    };
    match serde_json::from_str::<Value>(&answer) {
        Ok(response) if response.get("result").is_some() => {
            said(format!("{SCRIPT_LABEL}: {}", evaluated(&response)), false)
        }
        Ok(response) if response["error"].is_string() => {
            said(response["error"].as_str().unwrap_or_default().into(), true)
        }
        Ok(response) if response["dialog"] == true => said(DIALOG.into(), true),
        _ => said(
            "the page gave the script no answer: no page shows, it took too long or answered too much, or this \
            session's browser image predates scripts (a session on a newer one runs them)"
                .into(),
            true,
        ),
    }
}

/// A turn for an agent's call on the session's page or window (a reading, a script, a navigation, closing the
/// window), as a desktop call takes one: none in the demo, for a caller that may no longer act or reach the session,
/// or with too many calls waiting.
pub fn admitted<'a>(app: &App, who: Owner, session: &'a Session) -> Result<crate::agent::Turn<'a>, Called> {
    if app.config.demo.is_some() {
        return Err(Called::Failed("not in the demo".into()));
    }
    if !app.may_act(who) {
        return Err(Called::Revoked);
    }
    if !app.reachable(who, session) {
        return Err(Called::Unreached);
    }
    session.agent.place().ok_or(Called::Busy)
}

/// Asks the session's page, in the turn [`admitted`] gave (see `Agent::script`): after the call before it, never
/// while a person holds control, and given up before a takeover gets control; whether its caller may still act is
/// asked again once its turn comes. One question of the page at a time: a reading of it (`inspect_page`) under way
/// ends first, unless a takeover comes meanwhile, and then nothing was asked. `ask` asks the display at its socket,
/// told of a takeover, and says what it answered and whether the browser is done with it. Its answer, or how the
/// call ends without one.
async fn asked<F>(
    app: &App,
    who: Owner,
    session: &Session,
    ask: impl FnOnce(std::path::PathBuf, crate::agent::Stopped) -> F + Send + 'static,
) -> Result<Result<String, String>, Called>
where
    F: std::future::Future<Output = (Result<String, String>, bool)> + Send,
{
    let path = app.config.streams_dir.join(&session.id).join("display.sock");
    let alone = session.inspecting.clone();
    let exchange = move |stopped: crate::agent::Stopped| async move {
        let _alone = tokio::select! {
            biased;
            () = stopped.wait() => return (Ok(String::new()), true),
            alone = alone.lock_owned() => alone,
        };
        ask(path, stopped).await
    };
    let still = || app.may_act(who) && app.reachable(who, session);
    session.agent.script(still, exchange).await.map_err(|error| crate::service::refused(app, who, error))
}

/// Runs a `run_javascript` for an agent ([`admitted`], [`asked`]).
pub async fn run(app: &App, who: Owner, session: &Session, arguments: Value) -> Called {
    let _turn = match admitted(app, who, session) {
        Ok(turn) => turn,
        Err(called) => return called,
    };
    let javascript = match script(session.devtools, arguments) {
        Ok(javascript) => javascript,
        Err(why) => return Called::Failed(why),
    };
    let ask = move |path: std::path::PathBuf, stopped: crate::agent::Stopped| async move {
        ask_script(&path, &javascript, &stopped).await
    };
    match asked(app, who, session, ask).await {
        Ok(answer) => Called::Done(scripted(answer)),
        Err(called) => called,
    }
}

/// Sends the display whose stream socket is `path` a script, on a link of its own, in parts cut where characters
/// end, and returns what the browser answered, and whether the script is settled: it answered (a `Scripted` came,
/// with an answer or, for a script given up, none), or nothing of the script was sent. Once `stopped` says a takeover
/// began, the display is told to give the script up, and its `Scripted` is still waited for: after it, a script not
/// yet written to the browser never is (one already written may still run there, as the launcher's `stop_script`
/// says). An exchange that fails after it sent anything, or runs out of time, says the script is not known to be
/// settled.
async fn ask_script(path: &Path, javascript: &str, stopped: &crate::agent::Stopped) -> (Result<String, String>, bool) {
    let sent = std::sync::atomic::AtomicBool::new(false);
    let exchange = async {
        let mut link = DisplayLink::open(path).await?;
        let mut rest = javascript;
        loop {
            let (part, after) = rest.split_at(rest.floor_char_boundary(MAX_SCRIPT_PART));
            sent.store(true, std::sync::atomic::Ordering::Relaxed);
            link.send(&Link::Script { last: after.is_empty(), part: part.to_owned() }).await?;
            if after.is_empty() {
                break;
            }
            rest = after;
        }
        let scripted = |link| if let Link::Scripted { answer } = link { Some(answer) } else { None };
        answer_or_stop(&mut link, stopped, &std::sync::atomic::AtomicBool::new(false), &Link::ScriptStop, scripted)
            .await
    };
    let answer = tokio::time::timeout(SCRIPT_WAIT + SCRIPT_MARGIN, exchange)
        .await
        .unwrap_or_else(|_| Err("the script did not answer in time".into()));
    let done = answer.is_ok() || !sent.load(std::sync::atomic::Ordering::Relaxed);
    (answer, done)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Going {
    url: String,
    read: Option<Value>,
}

fn going_usage() -> String {
    format!("{NAVIGATE} takes {{url: an http(s) address or about:blank, read?: {{selector, html?, limit?}}}}")
}

/// What reads the page shown and nothing of it: enough for a navigation to say where it is. No element: reading one,
/// even to its first character, has the page lay itself out and gather all its text.
const NO_READ: &str = r#"{"selector":"html","html":false,"limit":0,"text":0,"attribute":0}"#;

/// The navigation as the launcher takes it, from what an agent sent: its address, as a session's start may be, and
/// what to read there (the agent's question, or none of the page). With what the agent asked to have read.
fn going(arguments: Value, held: usize) -> Result<(String, String, Reads), String> {
    let asked: Going = serde_json::from_value(arguments).map_err(|_| going_usage())?;
    let url = asked.url.trim().to_owned();
    display_protocol::session::start_url(&url).map_err(|why| format!("{NAVIGATE}: the url {why}"))?;
    let (read, reads) = match asked.read {
        Some(read) => (question(read, held).map_err(|_| going_usage())?, Reads::Elements),
        None => (NO_READ.to_owned(), Reads::Nothing),
    };
    let read: Value = serde_json::from_str(&read).map_err(|_| going_usage())?;
    let request = json!({"url": url, "read": read}).to_string();
    if request.len() > MAX_NAVIGATE { Err(going_usage()) } else { Ok((url, request, reads)) }
}

/// What a navigation's agent asked to have read of the page it goes to.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Reads {
    Nothing,
    Elements,
}

/// What an agent gets of a navigation's answer from the display (see the launcher's `Pages::navigate`): what became
/// of it, where the page is then, and what was read of it there when the agent asked. `given_up` says the admin
/// had it given up for a takeover. A navigation that did not get there is an error result that says how far it got
/// (`began`: whether the browser had begun a new document for it, so the page may have left the one it was on),
/// and so is one whose page was not read as asked, or did not say where it is (`read_error` says why): the
/// navigation is told all the same, with what the page did say of where it is.
fn navigated(
    answer: Result<String, String>,
    url: &str,
    reads: Reads,
    held: usize,
    given_up: bool,
) -> Result<Value, String> {
    let answer = answer?;
    let gone = || "the browser gave the navigation no answer: it may have gone, or not".to_owned();
    let said: Value = serde_json::from_str(&answer).map_err(|_| gone())?;
    let outcome = match said["navigation"]["outcome"].as_str().ok_or_else(gone)? {
        "unsupported" => {
            return Err("this session's browser image predates navigation (a session on a newer one navigates): go \
                there with the keyboard"
                .into());
        }
        "no_page" => return Err("no page shows: there is no tab to take there".into()),
        "starting" => return Err("this session's browser is not ready yet: try again in a moment".into()),
        "ambiguous" => {
            return Err("which page shows could not be told (more than one window shows a page and the keyboard \
                is in none of them, or a page did not answer): click in the one to use, or try again"
                .into());
        }
        // Given up for time (the display's, when the launcher heard nothing from the browser), not for a person.
        "stopped" if !given_up => "timeout",
        other => other,
    };
    let arrived = matches!(outcome, "loaded" | "same_document");
    let mut navigation = json!({"requested": url, "outcome": outcome});
    if let Some(error) = said["navigation"]["error"].as_str() {
        navigation["error"] = json!(error);
    } else if outcome == "dialog" {
        navigation["error"] = json!(DIALOG_NAVIGATION);
    }
    if !arrived && outcome != "download" {
        navigation["began"] = json!(said["navigation"]["began"] == true);
    }
    // Where the page is, which it says even when it could not be read as asked.
    let mut read = said.get("read").filter(|read| read.is_object()).cloned();
    for field in ["url", "title", "status"] {
        let value = read.as_mut().and_then(Value::as_object_mut).and_then(|read| read.remove(field));
        if let Some(value) = value.filter(|value| !value.is_null()) {
            navigation[field] = value;
        }
    }
    let unread = || {
        "the page gave the reading no answer (it may have taken too long, or left for another \
        page): read it with inspect_page"
            .to_owned()
    };
    let (mut page, mut read_error) = match read.map(|read| answered_page(read, held)) {
        Some(Ok(page)) => (page, None),
        Some(Err(why)) => (json!({}), Some(why)),
        None => (json!({}), None),
    };
    let asked = reads != Reads::Nothing;
    if arrived && read_error.is_none() && ((asked && page.get("count").is_none()) || navigation.get("url").is_none()) {
        read_error = Some(unread());
    }
    if let Some(fields) = page.as_object_mut() {
        if !asked || read_error.is_some() {
            fields.clear();
        }
        fields.insert("navigation".into(), navigation);
        if let Some(why) = &read_error {
            fields.insert("read_error".into(), json!(why));
        }
    }
    let failed = !(arrived || outcome == "download") || read_error.is_some();
    let mut result = result(page);
    result["isError"] = json!(failed);
    Ok(result)
}

/// How long a navigation waits for a session's browser to have a page to take somewhere, and how often it asks.
const READY_WAIT: Duration = Duration::from_secs(15);
const READY_EVERY: Duration = Duration::from_millis(300);

/// Whether the display's answer says there was nothing to navigate yet: no browser, or no page of it.
fn not_ready(answer: &Result<String, String>) -> bool {
    let said = answer.as_ref().ok().and_then(|answer| serde_json::from_str::<Value>(answer).ok());
    matches!(said.as_ref().and_then(|said| said["navigation"]["outcome"].as_str()), Some("starting" | "no_page"))
}

/// How a navigation ended, for its receipt (see `calls`), from the display's answer: as the result its caller gets
/// says it ([`navigated`]), so a page that was reached and not read as asked is an error there too.
fn ended(answer: &Result<String, String>, url: &str, reads: Reads, held: usize) -> crate::calls::Outcome {
    use crate::calls::Outcome;
    let Ok(text) = answer else { return Outcome::Failed };
    let said = serde_json::from_str::<Value>(text).ok();
    if said.as_ref().and_then(|said| said["navigation"]["outcome"].as_str()).is_none() {
        return Outcome::Unknown;
    }
    match navigated(Ok(text.clone()), url, reads, held, false) {
        Ok(result) if result["isError"] != true => Outcome::Ok,
        _ => Outcome::Error,
    }
}

/// Takes a session's page to an address for an agent (`navigate`), in a turn as a desktop call takes one (see
/// `Agent::script`): after the call before it, never while a person holds control, and given up before a takeover
/// gets control (a navigation the browser already began goes on: it cannot be taken back). Whether its caller may
/// still act is asked again once its turn comes. From when it reaches the display (its link is open) until the
/// browser's answer, it is among the session's running calls, and `receipt` (its `call_id`'s, see `calls`) says how
/// it ended: a caller that left meanwhile, and sends it again, is told so and the page is not taken there twice. A
/// navigation that never reached the display gives its receipt back.
pub async fn navigate(
    app: &App,
    who: Owner,
    session: &Session,
    arguments: Value,
    receipt: Option<crate::calls::Reservation>,
    caller: &Value,
) -> Called {
    let _turn = match admitted(app, who, session) {
        Ok(turn) => turn,
        Err(called) => return called,
    };
    let held = session.state.lock().unwrap().redactions.held();
    let (url, request, reads) = match going(arguments, held) {
        Ok(going) => going,
        Err(why) => return Called::Failed(why),
    };
    let stopping = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let told = stopping.clone();
    let (calls, by, at) = (session.agent.calls.clone(), caller.clone(), url.clone());
    let ask = move |path: std::path::PathBuf, stopped: crate::agent::Stopped| async move {
        // Among the running calls, and its receipt kept, only from when the display has it: one that never
        // reached the display gives its receipt back, and may be sent again.
        let (mut running, mut receipt) = (None, Some(receipt));
        let mut reached = || {
            if let Some(receipt) = receipt.take() {
                running = Some(calls.started(NAVIGATE, &by, receipt));
            }
        };
        // A session that just started may have no browser to ask yet, or a browser with no page yet: nothing was
        // navigated then, and it is asked again for a while, unless a takeover comes.
        let began = std::time::Instant::now();
        let (answer, done) = loop {
            let asked = ask_navigate(&path, request.clone(), &stopped, &told, &mut reached).await;
            if !not_ready(&asked.0) || began.elapsed() >= READY_WAIT {
                break asked;
            }
            tokio::select! {
                biased;
                () = stopped.wait() => break asked,
                () = tokio::time::sleep(READY_EVERY) => {}
            }
        };
        if let Some(running) = running {
            running.end(ended(&answer, &at, reads, held));
        }
        (answer, done)
    };
    match asked(app, who, session, ask).await {
        Ok(answer) => {
            let given_up = stopping.load(std::sync::atomic::Ordering::Relaxed);
            match navigated(answer, &url, reads, held, given_up) {
                Ok(result) => Called::Done(result),
                Err(why) => Called::Failed(why),
            }
        }
        Err(called) => called,
    }
}

/// Sends the display whose stream socket is `path` a navigation, on a link of its own, and returns what became of
/// it, and whether the browser is done with it: a `Navigated` came, or nothing was sent. Once `stopped` says a
/// takeover began, the display is told to give the navigation up (`told` then says so), and its `Navigated` is
/// still waited for: only after it does nothing of the navigation begin. `reached` is called once the display is
/// reached, before anything is sent to it: from there the navigation may have happened.
async fn ask_navigate(
    path: &Path,
    request: String,
    stopped: &crate::agent::Stopped,
    told: &std::sync::atomic::AtomicBool,
    mut reached: impl FnMut(),
) -> (Result<String, String>, bool) {
    use std::sync::atomic::Ordering;
    let sent = std::sync::atomic::AtomicBool::new(false);
    let exchange = async {
        let mut link = DisplayLink::open(path).await?;
        sent.store(true, Ordering::Relaxed);
        reached();
        link.send(&Link::Navigate { request }).await?;
        let navigated = |link| if let Link::Navigated { answer } = link { Some(answer) } else { None };
        answer_or_stop(&mut link, stopped, told, &Link::NavigateStop, navigated).await
    };
    let answer = tokio::time::timeout(NAVIGATE_WAIT + SCRIPT_MARGIN, exchange)
        .await
        .unwrap_or_else(|_| Err("the navigation did not answer in time".into()));
    let done = answer.is_ok() || !sent.load(Ordering::Relaxed);
    (answer, done)
}

/// Waits on `link` for the answer `pick` finds among what the display sends. Once `stopped` says a takeover began,
/// the display is told `stop` (`told` then says so), and the answer is still waited for: after it, the launcher
/// writes no script's evaluation or navigation of what was asked (what the browser was given before is as
/// `ScriptStop` and `NavigateStop` say; a reading of the page, which changes nothing, may still be on its way).
async fn answer_or_stop(
    link: &mut DisplayLink,
    stopped: &crate::agent::Stopped,
    told: &std::sync::atomic::AtomicBool,
    stop: &Link,
    pick: impl Fn(Link) -> Option<String>,
) -> Result<String, String> {
    use std::sync::atomic::Ordering;
    loop {
        tokio::select! {
            biased;
            () = stopped.wait(), if !told.load(Ordering::Relaxed) => {
                told.store(true, Ordering::Relaxed);
                link.send(stop).await?;
            }
            read = link.read() => {
                if let Some(answer) = read?.into_iter().find_map(&pick) {
                    return Ok(answer);
                }
            }
        }
    }
}

/// What the admin waits past the display's own bounds for a script's answer: the display's wait for the launcher to
/// give a script up that took too long, and a little more.
const SCRIPT_MARGIN: Duration = Duration::from_secs(12);

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
        "{NAME} takes {{selector: a CSS selector of at most {MAX_SELECTOR} bytes, html?: true, limit?: 1 to \
         {MAX_ELEMENTS}}}"
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
    let inspected = |link| if let Link::Inspected { answer } = link { Some(answer) } else { None };
    DisplayLink::ask(path, &Link::Inspect { query }, inspected, ANSWER_DEADLINE, "the page did not answer in time")
        .await
}

/// What an agent is told while a dialog of a page is open (the launcher's `Pages::dialog`): the page answers nothing
/// until it is closed, and the window's tree shows it with its buttons. In the admin's own words, with nothing of the
/// page's in them.
const DIALOG: &str = "a dialog of the page is open in the browser's window (an alert, a question, or whether to \
    leave the page), and the page answers nothing until it is closed: read the window (get_window_state), then click \
    one of the dialog's buttons. If the window shows none, the dialog is another tab's";
/// As [`DIALOG`], for a navigation: the dialog may be the page asking whether to leave it for the address, which the
/// launcher then tries to stop (see the launcher's `Pages::dialog`).
const DIALOG_NAVIGATION: &str = "a dialog of the page is open in the browser's window (an alert, a question, or \
    whether to leave the page), and the navigation was given up (`began` says whether the browser had begun \
    loading a new page, which may go on): read the window (get_window_state), answer the dialog with one of its buttons, and read the \
    page to see where it is. A page that asks whether to leave it asks again on every navigation: to leave it, type \
    the address in the browser's address bar and choose the dialog's button to leave. If the window shows none, the \
    dialog is another tab's";

/// What an agent gets of what the page answered. An empty answer is the browser's: no page, or too much of one.
fn answered(answer: &str, held: usize) -> Result<Value, String> {
    if answer.is_empty() {
        return Err("the page did not answer: the browser may have no page ready yet (try again in a moment), a tab \
            behind may be too busy to answer (close the tabs you do not need), or the reading was too much for it (try \
            a narrower selector, or a smaller limit)"
            .into());
    }
    let value: Value = serde_json::from_str(answer).map_err(|_| "the page's answer was not readable".to_owned())?;
    answered_page(value, held)
}

/// As [`answered`], of the page's answer already read as JSON.
fn answered_page(mut value: Value, held: usize) -> Result<Value, String> {
    if value["dialog"] == true {
        return Err(DIALOG.into());
    }
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

/// Makes each element's text and attributes what an agent may read of them up to their bounds, with what a person
/// typed out of them ([`crate::redact::Scrub::head`]), before the redaction reads the answer whole (see `service`):
/// while the answer still has the names the reading gave it, which the redaction may change, and so that nothing of
/// what the page sent past a bound (see [`question`]) is anywhere in it by then. The answer's text form is left
/// empty: [`trimmed`], which always follows, writes it from what the redaction left, so the redaction never reads the
/// whole page twice.
pub fn heads(scrub: &mut crate::redact::Scrub, result: &mut Value) {
    let Some(page) = result.get_mut("structuredContent") else { return };
    if page.get("elements").is_some_and(Value::is_array) {
        bounded(page, |value, most, _| {
            if let Some(text) = value.as_str() {
                *value = Value::String(scrub.head(text, most));
            }
        });
    }
    if let Some(first) = result.pointer_mut("/content/0/text") {
        *first = Value::String(String::new());
    }
}

/// What an agent gets of a redacted answer: each element's text and attributes cut to their bounds, only now that what a
/// person typed is out of them ([`heads`] left of each what its bound allows and the marks of what they typed, which
/// may be longer than what they stand for).
pub fn trimmed(mut result: Value) -> Value {
    let Some(page) = result.get_mut("structuredContent") else { return result };
    bounded(page, |value, most, text| {
        if let Some(trimmed) = value.as_str().filter(|_| text).map(|s| s.trim().to_owned()) {
            *value = Value::String(trimmed);
        }
        cut(Some(value), most);
    });
    let text = page.to_string();
    if let Some(first) = result.pointer_mut("/content/0/text") {
        *first = Value::String(text);
    }
    result
}

/// Each element's fields a reading bounds, its text and its attributes' values, with its bound and whether it is the
/// text: [`heads`] and [`trimmed`] both go through these, so that every field cut is one what a person typed was taken
/// out of first.
fn bounded(page: &mut Value, mut each: impl FnMut(&mut Value, usize, bool)) {
    let Some(elements) = page.get_mut("elements").and_then(Value::as_array_mut) else { return };
    for element in elements {
        if let Some(text) = element.get_mut("text") {
            each(text, MAX_TEXT, true);
        }
        if let Some(attributes) = element.get_mut("attributes").and_then(Value::as_object_mut) {
            attributes.values_mut().for_each(|value| each(value, MAX_ATTRIBUTE, false));
        }
    }
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
    // A place among the session's calls, and one that counts as using the session. A reading does not wait for the
    // driver's call under way (it changes nothing); only other questions of the page (`_alone`) come before it.
    let _turn = match admitted(app, who, session) {
        Ok(turn) => turn,
        Err(called) => return called,
    };
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
        Ok(mut value) => {
            // What a navigation says of the page it got to, not a reading.
            if let Some(fields) = value.as_object_mut() {
                fields.remove("status");
            }
            Called::Done(result(value))
        }
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
    fn a_navigation_takes_an_address_a_session_may_start_at_and_what_to_read_there() {
        let (url, request, reads) = going(json!({"url": " https://e.test/a?b=1 "}), 0).unwrap();
        let request: Value = serde_json::from_str(&request).unwrap();
        assert_eq!((url.as_str(), reads), ("https://e.test/a?b=1", Reads::Nothing));
        assert_eq!((&request["url"], &request["read"]["limit"]), (&json!("https://e.test/a?b=1"), &json!(0)));
        let (_, request, reads) =
            going(json!({"url": "about:blank", "read": {"selector": "a", "limit": 3}}), 4).unwrap();
        let request: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(reads, Reads::Elements);
        assert_eq!(
            request["read"],
            json!({"selector": "a", "html": false, "limit": 3, "text": MAX_TEXT + 4, "attribute": MAX_ATTRIBUTE + 4})
        );
        for bad in [
            json!({}),
            json!({"url": "file:///etc/passwd"}),
            json!({"url": "javascript:alert(1)"}),
            json!({"url": "https://"}),
            json!({"url": "https://e.test/a b"}),
            json!({"url": "https://e.test/", "read": {"limit": 3}}),
            json!({"url": "https://e.test/", "read": "a"}),
            json!({"url": "https://e.test/", "selector": "a"}),
            json!({"url": "https://e.test/", "read": {"selector": "li", "fields": {"name": {}}}}),
            json!({"url": format!("https://e.test/{}", "a".repeat(display_protocol::session::MAX_URL))}),
        ] {
            assert!(going(bad.clone(), 0).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_navigation_answers_where_the_page_is_and_what_was_read_of_it_only_when_it_got_there() {
        let page = json!({
            "visible": true, "focused": true, "url": "https://e.test/b", "title": "B", "status": 200, "count": 1,
            "truncated": false, "elements": [{"tag": "a", "attributes": {}, "text": "x", "shown": true}],
        });
        let answer = |outcome: &str, began: bool, read: Option<&Value>| {
            Ok(json!({"navigation": {"outcome": outcome, "error": null, "began": began}, "read": read}).to_string())
        };
        let at = "https://e.test/a";
        let there =
            json!({"requested": at, "outcome": "loaded", "url": "https://e.test/b", "title": "B", "status": 200});
        // Loaded and read: the reading as inspect_page gives it, with where the navigation ended.
        let got = navigated(answer("loaded", true, Some(&page)), at, Reads::Elements, 0, false).unwrap();
        let said = &got["structuredContent"];
        assert_eq!((&said["navigation"], &said["count"], &got["isError"]), (&there, &json!(1), &json!(false)));
        assert!(said.get("visible").is_none() && said.get("status").is_none() && said["elements"][0]["tag"] == "a");
        // No reading asked for: where it is, and nothing of the page.
        let got = navigated(answer("same_document", false, Some(&page)), at, Reads::Nothing, 0, false).unwrap();
        let said = got["structuredContent"].as_object().unwrap();
        assert_eq!(said.keys().collect::<Vec<_>>(), ["navigation"]);
        assert_eq!((&said["navigation"]["outcome"], &got["isError"]), (&json!("same_document"), &json!(false)));
        // A download leaves the page: not an error, and nothing read.
        let got = navigated(answer("download", false, None), at, Reads::Elements, 0, false).unwrap();
        let said = &got["structuredContent"];
        assert_eq!((&got["isError"], &said["navigation"]["outcome"]), (&json!(false), &json!("download")));
        assert!(said.get("read_error").is_none() && said["navigation"].get("began").is_none(), "{got}");

        // It got there and the page gave no answer: an error that keeps where it got, whether a reading was asked
        // for or only where the page is.
        for reads in [Reads::Elements, Reads::Nothing] {
            let got = navigated(answer("loaded", true, None), at, reads, 0, false).unwrap();
            let said = &got["structuredContent"];
            assert_eq!((&got["isError"], &said["navigation"]["outcome"]), (&json!(true), &json!("loaded")));
            assert!(said["read_error"].as_str().is_some_and(|why| why.contains("inspect_page")), "{got}");
        }
        // It was not read as asked (a selector that is none): the error says why, and where the page is stays.
        let wrong =
            json!({"url": "https://e.test/b", "title": "B", "status": 200, "error": "not a valid CSS selector"});
        let got = navigated(answer("loaded", true, Some(&wrong)), at, Reads::Elements, 0, false).unwrap();
        let said = &got["structuredContent"];
        assert_eq!((&got["isError"], &said["read_error"]), (&json!(true), &json!("not a valid CSS selector")));
        assert_eq!((&said["navigation"], said.get("elements")), (&there, None));

        // What did not get there is an error result that says how far it got.
        let failed = json!({"navigation": {"outcome": "failed", "error": "net::ERR_X", "began": false}, "read": null});
        let got = navigated(Ok(failed.to_string()), at, Reads::Elements, 0, false).unwrap();
        assert_eq!(got["isError"], true);
        let expected = json!({"requested": at, "outcome": "failed", "error": "net::ERR_X", "began": false});
        assert_eq!(got["structuredContent"], json!({"navigation": expected}));
        // Given up by the display for time reads as a timeout, with whether the browser had begun a new document;
        // given up for a person stays what it was.
        let got = navigated(answer("stopped", true, None), at, Reads::Nothing, 0, false).unwrap();
        let navigation = &got["structuredContent"]["navigation"];
        assert_eq!(
            (&got["isError"], &navigation["outcome"], &navigation["began"]),
            (&json!(true), &json!("timeout"), &json!(true))
        );
        let got = navigated(answer("stopped", false, None), at, Reads::Nothing, 0, true).unwrap();
        assert_eq!(got["structuredContent"]["navigation"]["outcome"], "stopped");
        let got = navigated(answer("crashed", true, None), at, Reads::Elements, 0, false).unwrap();
        assert_eq!(
            (&got["isError"], &got["structuredContent"]["navigation"]["outcome"]),
            (&json!(true), &json!("crashed"))
        );
        // A browser from before it navigated, no page, no telling which page, or no answer: said, never taken for
        // a navigation.
        let said_so = [
            ("unsupported", "predates navigation"),
            ("no_page", "no page"),
            ("starting", "not ready"),
            ("ambiguous", "could not be told"),
        ];
        for (outcome, word) in said_so {
            let why = navigated(answer(outcome, false, None), at, Reads::Nothing, 0, false).unwrap_err();
            assert!(why.contains(word), "{outcome}: {why}");
        }
        assert!(navigated(Ok(String::new()), at, Reads::Nothing, 0, false).is_err());
        assert!(navigated(Err("display: gone".into()), at, Reads::Nothing, 0, false).unwrap_err().contains("gone"));
    }

    #[test]
    fn a_navigation_is_asked_again_only_while_there_was_nothing_to_navigate() {
        let said = |outcome: &str| Ok(json!({"navigation": {"outcome": outcome}, "read": null}).to_string());
        assert!(not_ready(&said("starting")) && not_ready(&said("no_page")));
        for outcome in ["loaded", "failed", "timeout", "ambiguous", "unsupported", "stopped"] {
            assert!(!not_ready(&said(outcome)), "{outcome}");
        }
        assert!(!not_ready(&Ok(String::new())) && !not_ready(&Err("display: gone".into())));
    }

    #[test]
    fn a_navigation_s_receipt_says_how_it_ended() {
        use crate::calls::Outcome;
        let page = json!({"url": "https://e.test/a", "title": "A", "status": 200, "count": 1, "truncated": false, "elements": []});
        let said = |outcome: &str, read: Value| {
            Ok(json!({"navigation": {"outcome": outcome, "error": null, "began": true}, "read": read}).to_string())
        };
        let at = "https://e.test/a";
        for (answer, outcome) in [
            (said("loaded", page.clone()), Outcome::Ok),
            (said("same_document", page.clone()), Outcome::Ok),
            (said("download", Value::Null), Outcome::Ok),
            (said("failed", Value::Null), Outcome::Error),
            (said("timeout", Value::Null), Outcome::Error),
            (said("stopped", Value::Null), Outcome::Error),
            (said("no_page", Value::Null), Outcome::Error),
            // Reached, and not read as asked: an error, as the result its caller gets is.
            (said("loaded", Value::Null), Outcome::Error),
            (said("loaded", json!({"url": "https://e.test/a", "error": "not a valid CSS selector"})), Outcome::Error),
            (Ok(String::new()), Outcome::Unknown),
            (Err("display: gone".to_owned()), Outcome::Failed),
        ] {
            assert_eq!(ended(&answer, at, Reads::Elements, 0), outcome, "{answer:?}");
        }
    }

    #[test]
    fn a_script_runs_only_in_a_session_with_developer_tools() {
        let asked = json!({"javascript": "1+1", "pid": 3, "window_id": 4});
        assert!(script(false, asked.clone()).unwrap_err().contains("Developer tools"));
        assert_eq!(script(true, asked).unwrap(), "1+1");
        assert!(script(true, json!({"javascript": "1"})).is_err());
        assert!(script(true, json!({"javascript": "1", "pid": 3, "window_id": 4, "action": "click_element"})).is_err());
        let long = json!({"javascript": "x".repeat(MAX_SCRIPT + 1), "pid": 3, "window_id": 4});
        assert!(script(true, long).unwrap_err().contains("at most"));
    }

    #[test]
    fn a_script_s_answer_reads_as_the_driver_s_script_tool_gave_it() {
        let text = |result: Value| result["content"][0]["text"].as_str().unwrap().to_owned();
        let answered = |cdp: Value| scripted(Ok(json!({"result": cdp}).to_string()));
        // A value as JSON, a thing that is no value by what it is called, an exception by its description.
        let value = answered(json!({"result": {"type": "string", "value": "Inspect42"}}));
        assert_eq!(
            (text(value.clone()), value.get("isError")),
            ("cdp.runtime.evaluate.user_gesture: \"Inspect42\"".into(), None)
        );
        assert_eq!(
            text(answered(json!({"result": {"type": "number", "value": 2}}))),
            "cdp.runtime.evaluate.user_gesture: 2"
        );
        let named = answered(json!({"result": {"type": "object", "description": "HTMLDivElement"}}));
        assert_eq!(text(named), "cdp.runtime.evaluate.user_gesture: HTMLDivElement");
        let thrown = answered(json!({
            "result": {"type": "object"}, "exceptionDetails": {"exception": {"description": "ReferenceError: x is not defined"}},
        }));
        assert_eq!(text(thrown), "cdp.runtime.evaluate.user_gesture: exception: ReferenceError: x is not defined");
        // What the browser refused, no answer at all, and a display that failed, are errors that say why.
        for failed in [
            scripted(Ok(r#"{"error":"Cannot find context"}"#.into())),
            scripted(Ok(String::new())),
            scripted(Err("display: gone".into())),
        ] {
            assert_eq!(failed["isError"], true, "{failed}");
            assert!(!text(failed).is_empty());
        }
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
    fn an_open_dialog_is_said_to_whoever_asks_the_page_anything() {
        // A reading, a script, and a navigation that went nowhere for it, or reached its page and could not read it.
        let reading = answered(r#"{"dialog":true}"#, 0).unwrap_err();
        assert!(reading.contains("dialog") && reading.contains("get_window_state"), "{reading}");
        let script = scripted(Ok(r#"{"dialog":true}"#.into()));
        assert_eq!((&script["isError"], script["content"][0]["text"].as_str()), (&json!(true), Some(DIALOG)));
        let at = "https://e.test/b";
        let nowhere = json!({"navigation": {"outcome": "dialog", "error": null, "began": false}, "read": null});
        let got = navigated(Ok(nowhere.to_string()), at, Reads::Nothing, 0, false).unwrap();
        let said = &got["structuredContent"]["navigation"];
        assert_eq!(
            (&got["isError"], &said["outcome"], &said["began"]),
            (&json!(true), &json!("dialog"), &json!(false))
        );
        let why = said["error"].as_str().unwrap();
        assert!(why.contains("dialog") && why.contains("leave"), "{why}");
        let there =
            json!({"navigation": {"outcome": "loaded", "error": null, "began": true}, "read": {"dialog": true}});
        let got = navigated(Ok(there.to_string()), at, Reads::Elements, 0, false).unwrap();
        let page = &got["structuredContent"];
        assert_eq!((&got["isError"], &page["navigation"]["outcome"]), (&json!(true), &json!("loaded")));
        assert_eq!(page["read_error"], DIALOG);
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

    #[test]
    fn the_start_of_what_a_person_typed_at_the_end_of_a_text_the_page_cut_never_reaches_an_agent() {
        let mut typed = crate::redact::Redactions::default();
        for value in ["abcd", "hunter2secret", "text", "attributes"] {
            typed.keep(value);
        }
        let past = typed.held();
        // As the page sends them, cut where it was asked to (see `question`): what was typed quoted again and again,
        // the last time cut short, with a shorter thing they typed inside what is left of it.
        let quoted = |most: usize| -> String {
            let whole = "hunter2secret abcd ".repeat(most).chars().take(most + past - 9).collect::<String>();
            format!("{whole}abcdhunte")
        };
        let page = json!({"elements": [
            {"text": quoted(MAX_TEXT), "attributes": {"title": quoted(MAX_ATTRIBUTE), "alt": "You typed: hunter2secret"}},
            {"text": "You typed: hunter2secret abcd", "attributes": {}},
        ]});
        let mut asked = answered_page(page, past).map(result).unwrap();
        let mut scrub = typed.scrub();
        heads(&mut scrub, &mut asked);
        // Already nothing of it, before the redaction of the whole answer, which changes the names it knows.
        let said = asked.to_string();
        assert!(!said.contains("hun") && !said.contains("abcd"), "{}", &said[said.len() - 300..]);
        let clean = trimmed(typed.applied(asked));
        let said = clean.to_string();
        assert!(!said.contains("hun") && !said.contains("abcd"), "{}", &said[said.len() - 300..]);
        // What the redaction read stands as it was written to it: the names a person typed are marked, as anywhere.
        let elements = &clean["structuredContent"]["elements"];
        let first = elements[0].as_object().unwrap();
        assert!(
            !first.contains_key("text") && !first.contains_key("attributes"),
            "{:?}",
            first.keys().collect::<Vec<_>>()
        );
        assert!(
            first.values().any(|value| value.as_str().is_some_and(|text| text.starts_with("[redacted] [redacted]")))
        );
        // A text the page did not cut loses nothing but what a person typed.
        assert!(elements[1].to_string().contains("You typed: [redacted] [redacted]"), "{}", elements[1]);
    }
}
