//! What happened in a session, for the administrator who comes back to it: the agent's calls (what it asked for, in a
//! summary that keeps none of what it typed, and how each ended), who held control and why it changed, when a person
//! watched, the agent's files and the questions it asked, and the title of the page on screen.
//!
//! Each session keeps its timeline in memory, bounded ([`MAX_EVENTS`], [`MAX_BYTES`]: the oldest go first); a stopped
//! session's is kept, scrubbed, for up to [`RETAINED_FOR`] (see [`Runs`]), so a session its agent stopped at the end of
//! its task can still be read. Nothing of it reaches the disk, only the administrator reads it, and the public demo
//! keeps none ([`Timeline::off`]).
//!
//! What it keeps of an agent's call is an allowlist, never its arguments: the tool, a summary of the action (a click,
//! how many characters were typed, the named keys of a key press, a scroll's direction, the steps of a batch), how
//! long it took and how it ended, never what a tool said. File names and the labels a batch named its targets by can
//! still quote what a person typed: an answer scrubs exactly those ([`texts`], [`put_texts`]) with the session's
//! redactions, then cuts each to [`MAX_TEXT`] characters (cut after, so a secret cut short still matches whole), and a
//! text too long to keep whole is not kept. A page's title is kept as the screen shows it, cut ([`cut`]): the agent
//! and the replay see it as it is, and none is kept while a person holds control.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

/// Events a timeline keeps, at most; older ones are dropped.
pub const MAX_EVENTS: usize = 2000;
/// What a timeline's events take, at most, as JSON.
pub const MAX_BYTES: usize = 256 * 1024;
/// The longest text an answer shows (an error, a file's name, a title), in characters.
pub const MAX_TEXT: usize = 200;
/// The longest text an event keeps until it is scrubbed; a longer one is not kept.
const MAX_KEPT: usize = 2000;
/// How long a stopped session's timeline is kept at most, and how many of them.
pub const RETAINED_FOR: Duration = Duration::from_secs(24 * 60 * 60);
pub const MAX_RUNS: usize = 32;

/// Milliseconds since the Unix epoch, now.
pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// One session's events, in order.
pub struct Timeline {
    inner: Mutex<Inner>,
    /// Off in the public demo: nothing is kept.
    on: AtomicBool,
}

struct Inner {
    events: VecDeque<(Value, usize)>,
    /// The next number (from 1). Each event has a number of its own (`id`) and the number of its last change (`seq`),
    /// so a reader asks for what changed after the last `seq` it has, and replaces an event it has by its `id`.
    next: u64,
    bytes: usize,
    /// When the session started.
    started: u64,
}

impl Default for Timeline {
    fn default() -> Timeline {
        let inner = Inner { events: VecDeque::new(), next: 1, bytes: 0, started: now_ms() };
        Timeline { inner: Mutex::new(inner), on: AtomicBool::new(true) }
    }
}

impl Timeline {
    /// Keeps nothing from now on (the public demo's sessions).
    pub fn off(&self) {
        self.on.store(false, Ordering::Relaxed);
    }

    /// Adds an event of `kind` with `fields` (an object), now. A refused call like the one before it (blocked while a
    /// person holds control, say, which an agent may retry often) counts as that one again, changed now.
    pub fn push(&self, kind: &str, fields: Value) {
        if !self.on.load(Ordering::Relaxed) {
            return;
        }
        let at = now_ms();
        let mut inner = self.inner.lock().unwrap();
        let Inner { events, next, bytes, .. } = &mut *inner;
        if kind == "call"
            && matches!(fields["outcome"].as_str(), Some("blocked" | "busy"))
            && let Some((last, size)) = events.back_mut()
            && last["kind"] == "call"
            && last["tool"] == fields["tool"]
            && last["outcome"] == fields["outcome"]
        {
            last["times"] = json!(last["times"].as_u64().unwrap_or(1) + 1);
            last["until"] = json!(at);
            last["seq"] = json!(*next);
            *next += 1;
            let grown = last.to_string().len();
            *bytes = *bytes + grown - std::mem::replace(size, grown);
            return;
        }
        let mut event = Map::new();
        event.insert("id".into(), json!(*next));
        event.insert("seq".into(), json!(*next));
        event.insert("at".into(), json!(at));
        event.insert("kind".into(), json!(kind));
        if let Value::Object(fields) = fields {
            event.extend(fields);
        }
        let event = Value::Object(event);
        let size = event.to_string().len();
        *next += 1;
        *bytes += size;
        events.push_back((event, size));
        while events.len() > MAX_EVENTS || *bytes > MAX_BYTES {
            let Some((_, size)) = events.pop_front() else { break };
            *bytes -= size;
        }
    }

    /// The events changed after number `after`, the next number, and the first event still kept (those before it were
    /// dropped), as the API answers them before [`put_texts`].
    pub fn page(&self, after: u64) -> Value {
        let inner = self.inner.lock().unwrap();
        let first = inner.events.front().and_then(|(e, _)| e["id"].as_u64()).unwrap_or(inner.next);
        let events: Vec<&Value> =
            inner.events.iter().map(|(e, _)| e).filter(|e| e["seq"].as_u64().is_some_and(|s| s > after)).collect();
        json!({"events": events, "next": inner.next, "first": first, "started": inner.started})
    }
}

/// A call's outcome as the timeline says it: `ok`; `error`, the tool answered that it failed (`isError`); `failed`, it
/// did not answer; `left`, its caller left before it was done; or it was not run: `blocked` (a person holds control),
/// `busy` (too many calls wait), `withdrawn` (its caller may no longer act). Never what a tool said: an error can quote
/// its arguments (a key, a query, text to type).
pub fn call_event(tool: &str, action: &Value, outcome: &str, took: Option<Duration>) -> Value {
    let mut fields = json!({"tool": tool, "action": action, "outcome": outcome});
    if let Some(took) = took {
        fields["ms"] = json!(took.as_millis() as u64);
    }
    fields
}

/// Whether an MCP result says the tool failed (`isError`).
pub fn tool_failed(result: &Value) -> bool {
    result["isError"] == true
}

/// `text` as an event keeps it until it is scrubbed: none past [`MAX_KEPT`] characters, as a cut text could keep the
/// start of a secret the redactions only match whole.
pub fn kept(text: &str) -> String {
    if text.chars().count() > MAX_KEPT { String::new() } else { text.to_owned() }
}

/// `text` cut to `max` characters, an ellipsis marking the cut.
pub fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// The texts of a page that can quote what a person typed, in a fixed order: each event's error and name, the roles
/// and labels its action's targets were named by, and the page's own label.
pub fn texts(page: &Value) -> Vec<String> {
    let mut found = Vec::new();
    visit(&mut page.clone(), &mut |text| found.push(text.clone()));
    found
}

/// Puts `scrubbed` (what [`texts`] took, scrubbed, in the same order) back into `page`, each cut to [`MAX_TEXT`]
/// characters; none, or not as many, blanks them all (a redaction set that is full hides everything).
pub fn put_texts(page: &mut Value, scrubbed: Option<Vec<String>>) {
    let wanted = texts(page).len();
    let mut scrubbed = scrubbed.filter(|s| s.len() == wanted).map(Vec::into_iter);
    visit(page, &mut |text| {
        *text = scrubbed.as_mut().and_then(Iterator::next).map(|s| cut(&s, MAX_TEXT)).unwrap_or_default();
    });
}

/// Calls `each` on every text [`texts`] names, in its order.
fn visit(page: &mut Value, each: &mut impl FnMut(&mut String)) {
    let field = |value: &mut Value, key: &str, each: &mut dyn FnMut(&mut String)| {
        if let Some(Value::String(text)) = value.get_mut(key) {
            each(text);
        }
    };
    let target = |action: &mut Value, each: &mut dyn FnMut(&mut String)| {
        if let Some(target) = action.get_mut("target") {
            field(target, "role", each);
            field(target, "name", each);
        }
    };
    if let Some(events) = page.get_mut("events").and_then(Value::as_array_mut) {
        for event in events {
            for key in ["error", "name"] {
                field(event, key, each);
            }
            if let Some(action) = event.get_mut("action") {
                target(action, each);
                if let Some(steps) = action.get_mut("steps").and_then(Value::as_array_mut) {
                    for step in steps {
                        target(step, each);
                    }
                }
            }
        }
    }
    field(page, "label", each);
}

/// What an agent's action did, from an allowlist: never what it typed, a script's source, or anything not named here.
pub fn summary(tool: &str, arguments: &Value) -> Value {
    let a = arguments;
    match tool {
        "click" | "double_click" | "right_click" => {
            let mut s = json!({"click": tool});
            if let Some(count) = a["count"].as_u64().filter(|&c| c > 1) {
                s["count"] = json!(count.min(3));
            }
            if let Some(button) = a["button"].as_str().filter(|b| matches!(*b, "left" | "right" | "middle")) {
                s["button"] = json!(button);
            }
            with_target(s, a)
        }
        "type_text" | "set_value" => with_target(json!({"typed": chars(&a["text"]).or_else(|| chars(&a["value"]))}), a),
        "press_key" => json!({"keys": keys(a["key"].as_str().into_iter(), &a["modifiers"])}),
        "hotkey" => {
            let keys_sent = a["keys"].as_array().map(|k| k.iter().filter_map(Value::as_str).collect::<Vec<_>>());
            json!({"keys": keys(keys_sent.unwrap_or_default().into_iter(), &Value::Null)})
        }
        "scroll" => {
            let direction = a["direction"].as_str().filter(|d| matches!(*d, "up" | "down" | "left" | "right"));
            with_target(json!({"scroll": direction}), a)
        }
        "drag" | "move_cursor" => json!({"pointer": tool}),
        "list_windows" | "get_window_state" | "get_desktop_state" | "list_files" => json!({"looked": tool}),
        "run_steps" => {
            let steps = a["steps"].as_array().map(Vec::as_slice).unwrap_or_default();
            let steps: Vec<Value> = steps.iter().take(64).map(step).collect();
            json!({"steps": steps})
        }
        "run_script" => json!({"script": chars(&a["script"])}),
        "save_download" | "send_file" | "choose_files" => json!({"files": tool}),
        _ => json!({}),
    }
}

/// One step of a batch, as [`summary`] says its action.
fn step(step: &Value) -> Value {
    let (tool, args) = match step["tool"].as_str() {
        Some(tool) => (tool, &step["args"]),
        None => step
            .as_object()
            .and_then(|o| o.iter().find(|(k, _)| !matches!(k.as_str(), "wait_for" | "expect" | "timeout_ms")))
            .map_or(("", &Value::Null), |(k, v)| (k.as_str(), v)),
    };
    let known = matches!(
        tool,
        "click"
            | "double_click"
            | "right_click"
            | "set_value"
            | "type_text"
            | "press_key"
            | "hotkey"
            | "scroll"
            | "drag"
            | "move_cursor"
    );
    if !known {
        return json!({"check": true});
    }
    let mut s = summary(tool, args);
    s["tool"] = json!(tool);
    s
}

/// `s` with the role and name a batch step named its target by, when it did: a control's label, as the page shows it.
fn with_target(mut s: Value, a: &Value) -> Value {
    let role = a["role"].as_str().map(kept);
    let name = a["name"].as_str().map(kept);
    if role.is_some() || name.is_some() {
        s["target"] = json!({"role": role, "name": name});
    }
    s
}

/// How many characters a text argument has, not the text.
fn chars(value: &Value) -> Option<usize> {
    value.as_str().map(|t| t.chars().count())
}

/// The keys a timeline names: the modifiers, and keys that type nothing.
const NAMED_KEYS: &[&str] = &[
    "ctrl",
    "control",
    "shift",
    "alt",
    "option",
    "cmd",
    "command",
    "meta",
    "super",
    "enter",
    "return",
    "tab",
    "escape",
    "esc",
    "backspace",
    "delete",
    "space",
    "up",
    "down",
    "left",
    "right",
    "arrowup",
    "arrowdown",
    "arrowleft",
    "arrowright",
    "home",
    "end",
    "pageup",
    "pagedown",
    "insert",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
];

/// A key press as named keys, from an allowlist: a key that types nothing by its name; a printable key alone is
/// counted, not named (keys pressed one by one spell text), and named only in a shortcut (with a modifier other than
/// Shift); anything else is a key.
fn keys<'a>(keys: impl Iterator<Item = &'a str>, modifiers: &'a Value) -> Value {
    let all: Vec<&str> =
        modifiers.as_array().into_iter().flatten().filter_map(Value::as_str).chain(keys).take(6).collect();
    let named = |k: &str| NAMED_KEYS.iter().find(|n| n.eq_ignore_ascii_case(k)).copied();
    let shortcut = all.iter().any(|k| {
        matches!(named(k), Some("ctrl" | "control" | "alt" | "option" | "cmd" | "command" | "meta" | "super"))
    });
    let said: Vec<String> = all
        .iter()
        .map(|k| match named(k) {
            Some(name) => name.to_owned(),
            None if shortcut && k.chars().count() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => {
                k.to_ascii_lowercase()
            }
            None => "character".to_owned(),
        })
        .collect();
    json!(said)
}

/// A stopped session's timeline, scrubbed as it stopped, kept for up to [`RETAINED_FOR`].
pub struct Run {
    pub id: String,
    pub stopped: u64,
    pub until: Instant,
    /// Its events, and its `label`: what the session list showed of it (its saved session's name, or its page).
    pub page: Value,
    /// What its agent did on screen, when it was recorded (see `replay`): deleted with the run.
    pub recording: Option<std::sync::Arc<crate::replay::Recording>>,
}

impl Drop for Run {
    fn drop(&mut self) {
        if let Some(recording) = &self.recording {
            recording.delete();
        }
    }
}

/// The stopped sessions' timelines: at most [`MAX_RUNS`], each for up to [`RETAINED_FOR`] ([`Runs::prune`]).
#[derive(Default)]
pub struct Runs(Mutex<VecDeque<Run>>);

impl Runs {
    pub fn keep(&self, run: Run) {
        let mut runs = self.0.lock().unwrap();
        runs.retain(|r| r.until > Instant::now() && r.id != run.id);
        if runs.len() >= MAX_RUNS {
            runs.pop_front();
        }
        runs.push_back(run);
    }

    /// Drops the runs kept past their time (the admin calls it every few minutes, see `main`).
    pub fn prune(&self) {
        self.0.lock().unwrap().retain(|r| r.until > Instant::now());
    }

    /// The runs kept, newest first, without their events.
    pub fn list(&self) -> Vec<Value> {
        let mut runs = self.0.lock().unwrap();
        runs.retain(|r| r.until > Instant::now());
        runs.iter()
            .rev()
            .map(|r| json!({"id": r.id, "label": r.page["label"], "started": r.page["started"], "stopped": r.stopped}))
            .collect()
    }

    /// Run `id`'s events changed after `after`, if it is kept.
    pub fn page(&self, id: &str, after: u64) -> Option<Value> {
        let mut runs = self.0.lock().unwrap();
        runs.retain(|r| r.until > Instant::now());
        let run = runs.iter().find(|r| r.id == id)?;
        let mut page = run.page.clone();
        if let Some(events) = page["events"].as_array_mut() {
            events.retain(|e| e["seq"].as_u64().is_some_and(|s| s > after));
        }
        page["stopped"] = json!(run.stopped);
        Some(page)
    }

    /// Run `id`'s recording, if it is kept and was recorded.
    pub fn recording(&self, id: &str) -> Option<std::sync::Arc<crate::replay::Recording>> {
        let mut runs = self.0.lock().unwrap();
        runs.retain(|r| r.until > Instant::now());
        runs.iter().find(|r| r.id == id)?.recording.clone()
    }

    /// Forgets run `id` (the administrator deleted it).
    pub fn forget(&self, id: &str) -> bool {
        let mut runs = self.0.lock().unwrap();
        let before = runs.len();
        runs.retain(|r| r.id != id);
        runs.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_keeps_no_typed_text_script_key_or_unknown_field() {
        let typed = summary("type_text", &json!({"text": "hunter2 secret", "pid": 3}));
        assert_eq!(typed, json!({"typed": 14}));
        assert!(!summary("run_script", &json!({"script": "await type('pw')"})).to_string().contains("pw"));
        assert_eq!(summary("press_key", &json!({"key": "a"})), json!({"keys": ["character"]}));
        assert_eq!(summary("press_key", &json!({"key": "Enter"})), json!({"keys": ["enter"]}));
        assert_eq!(summary("press_key", &json!({"key": "hunter2"})), json!({"keys": ["character"]}));
        assert_eq!(summary("hotkey", &json!({"keys": ["Ctrl", "L"]})), json!({"keys": ["ctrl", "l"]}));
        assert_eq!(summary("hotkey", &json!({"keys": ["ctrl", "secret"]})), json!({"keys": ["ctrl", "character"]}));
        let batch = summary(
            "run_steps",
            &json!({"steps": [
                {"tool": "set_value", "args": {"role": "textbox", "name": "Password", "value": "hunter2"}},
                {"click": {"role": "button", "name": "Sign in"}},
                {"expect": [{"text": "Welcome hunter2"}]},
                {"wait_for": {"text": "hunter2"}, "tool": "made_up", "args": {"text": "hunter2"}},
            ]}),
        );
        let said = batch.to_string();
        assert!(!said.contains("hunter2"), "{said}");
        assert_eq!(batch["steps"][0]["typed"], 7);
        assert_eq!(batch["steps"][1]["target"]["name"], "Sign in");
        assert_eq!((&batch["steps"][2], &batch["steps"][3]), (&json!({"check": true}), &json!({"check": true})));
        assert_eq!(summary("made_up", &json!({"secret": "x"})), json!({}));
    }

    #[test]
    fn a_timeline_is_bounded_numbered_and_a_repeated_refusal_changes_its_line() {
        let timeline = Timeline::default();
        for _ in 0..3 {
            timeline.push("call", call_event("click", &json!({}), "blocked", None));
        }
        let page = timeline.page(0);
        let events = page["events"].as_array().unwrap();
        assert_eq!(
            (events.len(), &events[0]["times"], &events[0]["id"], &events[0]["seq"]),
            (1, &json!(3), &json!(1), &json!(3))
        );
        timeline.push("call", call_event("click", &json!({}), "blocked", None));
        assert_eq!(timeline.page(3)["events"][0]["times"], 4, "a reader that has it hears of its change");
        timeline.push("control", json!({"to": "agent", "reason": "gave"}));
        assert_eq!(timeline.page(4)["events"][0]["kind"], "control");
        for i in 0..MAX_EVENTS + 5 {
            timeline.push("page", json!({"title": format!("page {i}")}));
        }
        let page = timeline.page(0);
        assert_eq!(page["events"].as_array().unwrap().len(), MAX_EVENTS);
        assert!(page["first"].as_u64().unwrap() > 2, "the oldest were dropped");
        let off = Timeline::default();
        off.off();
        off.push("page", json!({"title": "x"}));
        assert_eq!(off.page(0)["events"], json!([]));
    }

    #[test]
    fn the_texts_go_back_scrubbed_then_cut_and_are_blanked_when_they_cannot() {
        let long = format!("{}hunter2", "a".repeat(MAX_TEXT - 3));
        let mut page = json!({
            "label": "Bank",
            "events": [
                {"kind": "file", "name": long},
                {"kind": "page", "title": "hunter2 - Search"},
                {"kind": "call", "error": "e", "action": {"steps": [{"target": {"role": "button", "name": "Go"}}]}},
            ],
        });
        let texts = texts(&page);
        assert_eq!(texts, [long.as_str(), "e", "button", "Go", "Bank"]);
        let scrubbed: Vec<String> = texts.iter().map(|t| t.replace("hunter2", "•••")).collect();
        put_texts(&mut page, Some(scrubbed));
        let name = page["events"][0]["name"].as_str().unwrap();
        assert!(!name.contains("hunt"), "scrubbed whole, then cut: {name}");
        assert_eq!(page["events"][1]["title"], "hunter2 - Search", "a title as the screen shows it");
        assert_eq!(page["events"][2]["action"]["steps"][0]["target"]["name"], "Go");
        put_texts(&mut page, None);
        assert_eq!((&page["label"], &page["events"][2]["error"]), (&json!(""), &json!("")));
    }

    #[test]
    fn a_failure_is_what_the_result_says_and_a_text_too_long_is_not_kept() {
        assert!(!tool_failed(&json!({"content": [{"type": "text", "text": "ok"}]})));
        assert!(tool_failed(&json!({"isError": true, "content": [{"type": "text", "text": "no such key hunter2"}]})));
        assert_eq!(kept(&"é".repeat(MAX_KEPT)), "é".repeat(MAX_KEPT));
        assert_eq!(kept(&"é".repeat(MAX_KEPT + 1)), "");
    }
}
