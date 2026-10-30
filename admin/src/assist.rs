//! What the admin does around the driver's calls where the pinned driver cannot act under this display, and where it
//! tells the session's viewers the agent acts.
//!
//! - Text beyond printable ASCII, tab and newline, which the driver's injection client refuses: a `type_text` of it
//!   goes to the driver without its text, so the driver targets the call as it targets any (focusing the element, or
//!   clicking the point, the window's binding checked) and answers as for text; the display then types the text with
//!   its `u` line into the same target, the process the driver names its window by, as it types a person's, and the
//!   agent gets the driver's answer with the count of what was typed.
//! - A click by element whose element has no action the driver fires here (a menu item that only selects): the driver
//!   resolved the element, then falls back to an X11 pointer, which this container has none of. The admin clicks the
//!   centre of the element's frame in the last snapshot it relayed instead, and answers as the driver's click does.
//! - A click by window pixels that the driver refuses because its latest read of that very window (it names it) had no
//!   screenshot: it captures none under this display, whose windows it cannot place. The admin clicks that pixel of the
//!   window. A window never read, read by another session, or not the one named, keeps the driver's refusal.
//!
//! Either click goes with the display's `c` line to the process the driver targets the window by, which the display
//! clicks only while that process's window is the one shown and what is under the point is its own.
//!
//! Each line goes on the display's injection link (see `inject`) under the rules of the agent's own lines, refused while
//! a person holds control, and only within the agent's call, which holds the agent's link: a takeover waits for it.
//!
//! Where the agent acts (a click's point, an element's centre, and for keys the last such place) goes to the session's
//! viewers alone, to draw the agent's cursor there: never into the composed picture, and never to the agent.

use std::collections::HashMap;
use std::path::PathBuf;

use display_protocol::MAX_TEXT;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::agent::CallError;

/// Element centres kept from the snapshots relayed: more than a walk returns (5 000 elements at most). Past it the
/// older snapshots' are dropped, whose tokens the driver has let go of by then.
const MAX_CENTRES: usize = 8192;
/// Snapshots whose process is kept: far more than the driver keeps (32), whose older tokens it has let go of.
const MAX_SNAPSHOTS: usize = 256;
/// What the driver answers a click by element that would need an X11 pointer.
const NO_X11: &str = "X11 connection failed";
/// The driver's refusal of a click by pixels with no screenshot of the window, and its reason when the window's latest
/// read had none.
const NO_SCREENSHOT: &str = "screenshot_context_missing";
const NO_SCREENSHOT_IN_READ: &str = "no_screenshot";
/// The path the display's own injection takes, as the driver names it in its answers.
const PATH: &str = "wayland_cua_compositor";

/// One session's: the display it injects into, what it learned of the snapshots, and where the agent last acted.
pub struct Assist {
    /// The display's stream socket.
    display: PathBuf,
    seen: std::sync::Mutex<Seen>,
    /// Where the agent last acted, in output pixels: each relay of the session watches it (see `relay`).
    place: watch::Sender<Option<(u16, u16)>>,
}

#[derive(Default)]
struct Seen {
    /// Each element token's frame centre, in output pixels: the driver rebases an element's frame onto the output.
    centres: HashMap<String, (f64, f64)>,
    /// The process each snapshot relayed was read of, by its id: what an element token's window is targeted by.
    pids: HashMap<String, u64>,
    /// The latest window and desktop screenshots' pixels per output pixel: the scale of the pixels the agent reads off
    /// each, which its window and desktop coordinates are in.
    window_scale: Option<f64>,
    desktop_scale: Option<f64>,
    last: Option<(f64, f64)>,
}

/// How a call goes to the driver, and what follows its answer.
#[derive(Debug, PartialEq)]
pub enum Route {
    Driver,
    /// A `type_text` sent without this text, which the display types into the process `pid` (the driver's target for
    /// the window) once the driver targeted it.
    Text {
        text: String,
        pid: u64,
    },
    /// A `type_text` of such text longer than the display types at once, refused before the driver is asked anything.
    TooLong,
}

impl Route {
    /// The answer a call of this route gets without the driver, if it gets one.
    pub fn refusal(&self) -> Option<Value> {
        (*self == Route::TooLong).then(|| {
            let why = format!(
                "cua-compositor rejected the command: text-too-long (at most {MAX_TEXT} bytes of text beyond printable \
                 ASCII per call; type it in several)"
            );
            json!({"content": [{"type": "text", "text": why}], "isError": true,
                "structuredContent": {"code": "text_too_long", "max_bytes": MAX_TEXT}})
        })
    }
}

impl Assist {
    pub fn new(display: PathBuf) -> Assist {
        Assist { display, seen: std::sync::Mutex::default(), place: watch::Sender::new(None) }
    }

    /// Where the agent acts, from now on.
    pub fn places(&self) -> watch::Receiver<Option<(u16, u16)>> {
        self.place.subscribe()
    }

    /// The call's route, its `arguments` changed for it (a text the display types is taken out). Text goes to the
    /// display only for a call whose process is known: the one it names, or the one its element's snapshot was read of.
    pub fn route(&self, tool: &str, arguments: &mut Value) -> Route {
        if tool != "type_text" || desktop(arguments) && arguments.get("pid").is_none() {
            // The driver types into the focused application itself, through its virtual keyboard, whatever the text.
            return Route::Driver;
        }
        let Some(pid) = self.pid(arguments) else { return Route::Driver };
        match arguments.get_mut("text") {
            Some(Value::String(text)) if beyond_driver(text) && text.len() > MAX_TEXT => Route::TooLong,
            Some(Value::String(text)) if beyond_driver(text) => Route::Text { text: std::mem::take(text), pid },
            _ => Route::Driver,
        }
    }

    /// The process a call targets: the one it names, else the one its element's snapshot was read of.
    fn pid(&self, arguments: &Value) -> Option<u64> {
        let named = arguments.get("pid").or_else(|| arguments.pointer("/target/pid")).and_then(Value::as_u64);
        let snapshot = || {
            let token = arguments.get("element_token")?.as_str()?;
            let id = token.split_once(':')?.0;
            self.seen.lock().unwrap().pids.get(id).copied()
        };
        named.or_else(snapshot).filter(|pid| (1..=u64::from(u32::MAX)).contains(pid))
    }

    /// The answer the agent gets for its call: the driver's `result`, after what the route asks of the display, or in
    /// place of a refusal the display can do better. `arguments` are the call's as the agent sent them, for the tools
    /// that act at a place (see [`acts`]); `blocked` says whether a person holds control (or takes it).
    pub async fn answered(
        &self,
        blocked: impl Fn() -> bool,
        tool: &str,
        arguments: Option<&Value>,
        route: Route,
        result: Value,
    ) -> Result<Value, CallError> {
        let empty = Value::Null;
        let arguments = arguments.unwrap_or(&empty);
        let mut result = result;
        if !failed(&result) {
            if let Route::Text { text, pid } = route {
                let hex: String = text.bytes().map(|b| format!("{b:02x}")).collect();
                // The target the driver names a window by its process with, so the text goes nowhere else.
                if let Err(why) = self.inject(&blocked, format!("u root:{pid} {hex}")).await? {
                    return Ok(refused(&why));
                }
                result = typed(result, text.chars().count());
            }
        } else if tool == "click"
            && let Some(fallback) = self.fallback(arguments, &result)
            && let Some(button) = button(arguments)
        {
            let Fallback { pid, frame, at: (x, y), element } = fallback;
            let count = count(arguments);
            if let Err(why) = self.inject(&blocked, format!("c root:{pid} {frame} {x} {y} {count} {button}")).await? {
                return Ok(refused(&why));
            }
            result = clicked(arguments, (x, y), count, element);
        }
        if failed(&result) {
            return Ok(result);
        }
        if matches!(tool, "get_window_state" | "get_desktop_state") {
            self.remember(tool, arguments, &result);
        } else if let Some(at) = self.acted(tool, arguments, &result) {
            self.seen.lock().unwrap().last = Some(at);
            let clamp = |v: f64| v.round().clamp(0.0, f64::from(u16::MAX)) as u16;
            self.place.send_replace(Some((clamp(at.0), clamp(at.1))));
        }
        Ok(result)
    }

    /// Sends one line to the display unless a person holds control: its refusal (`err` and why) is the inner error,
    /// and control taken from the agent meanwhile blocks the call, as any call of it is then.
    async fn inject(&self, blocked: &impl Fn() -> bool, line: String) -> Result<Result<(), String>, CallError> {
        if blocked() {
            return Err(CallError::Blocked);
        }
        let answer = crate::inject::send(&self.display, line).await.map_err(CallError::Failed)?;
        match answer.strip_prefix("err").map(str::trim) {
            None if answer == "ok" => Ok(Ok(())),
            Some("control-held") => Err(CallError::Blocked),
            Some(why) => Ok(Err(why.to_owned())),
            None => Ok(Err(format!("unexpected answer {answer:?}"))),
        }
    }

    /// Learns a snapshot's element centres, the process it was read of (from the call's `arguments`), and the scale
    /// of its screenshot. The driver says a window's `frame_scale` as the screenshot's pixels per window pixel, and the
    /// desktop's the other way round, output pixels per screenshot pixel.
    fn remember(&self, tool: &str, arguments: &Value, result: &Value) {
        let Some(structured) = result.get("structuredContent") else { return };
        let pid = arguments.get("pid").or_else(|| arguments.pointer("/target/pid")).and_then(Value::as_u64);
        let mut seen = self.seen.lock().unwrap();
        if let (Some(id), Some(pid)) = (structured.get("snapshot_id").and_then(Value::as_str), pid) {
            if seen.pids.len() >= MAX_SNAPSHOTS {
                seen.pids.clear();
            }
            seen.pids.insert(id.to_owned(), pid);
        }
        if let Some(scale) = structured.get("frame_scale").and_then(Value::as_f64).filter(|s| s.is_finite() && *s > 0.0)
        {
            match tool {
                "get_desktop_state" => seen.desktop_scale = Some(1.0 / scale),
                _ => seen.window_scale = Some(scale),
            }
        }
        let Some(elements) = structured.get("elements").and_then(Value::as_array) else { return };
        if seen.centres.len() + elements.len() > MAX_CENTRES {
            seen.centres.clear();
        }
        for element in elements.iter().take(MAX_CENTRES) {
            if let (Some(token), Some(centre)) = (element.get("element_token").and_then(Value::as_str), centre(element))
            {
                seen.centres.insert(token.to_owned(), centre);
            }
        }
    }

    /// Where a click the driver refused can still go (see the module's docs): an element's centre when the driver
    /// resolved the element but lacked an X11 pointer for it, or the window pixel when the driver's latest read of the
    /// window it names had no screenshot; each to the process the window is targeted by.
    fn fallback(&self, arguments: &Value, result: &Value) -> Option<Fallback> {
        if arguments.get("modifier").and_then(Value::as_array).is_some_and(|m| !m.is_empty()) {
            return None;
        }
        if let Some(token) = arguments.get("element_token").and_then(Value::as_str) {
            if !text(result).contains(NO_X11) {
                return None;
            }
            let pid = self.pid(arguments)?;
            let at = *self.seen.lock().unwrap().centres.get(token)?;
            return Some(Fallback { pid, frame: 'o', at, element: Some(token.to_owned()) });
        }
        let refusal = result.get("structuredContent")?;
        let said = |key: &str| refusal.get(key).and_then(Value::as_str);
        if said("code") != Some(NO_SCREENSHOT) || said("reason") != Some(NO_SCREENSHOT_IN_READ) {
            return None;
        }
        if desktop(arguments) || arguments.get("from_zoom").and_then(Value::as_bool) == Some(true) {
            return None;
        }
        // The window the driver read is the one the call names.
        let named = |key: &str| arguments.get(key).or_else(|| arguments.pointer(&format!("/target/{key}")));
        let pid = refusal.get("pid").and_then(Value::as_u64)?;
        let window = refusal.get("window_id").and_then(Value::as_u64)?;
        if named("pid").and_then(Value::as_u64).is_some_and(|p| p != pid)
            || named("window_id").and_then(Value::as_u64) != Some(window)
            || !(1..=u64::from(u32::MAX)).contains(&pid)
        {
            return None;
        }
        let at = |key: &str| arguments.get(key).and_then(Value::as_f64).filter(|v| v.is_finite());
        Some(Fallback { pid, frame: 'w', at: (at("x")?, at("y")?), element: None })
    }

    /// Where a call that succeeded acted, in output pixels, if it acts at a place (see [`acts`]): the point the driver
    /// says it resolved, else the element's centre or the pixels the agent named (read off the scale of its latest
    /// screenshot of the window or of the desktop, whichever the pixels are of), else, for keys, the last place.
    fn acted(&self, tool: &str, arguments: &Value, result: &Value) -> Option<(f64, f64)> {
        if !acts(tool) {
            return None;
        }
        let seen = self.seen.lock().unwrap();
        let resolved = result
            .pointer("/structuredContent/screen_point")
            .and_then(Value::as_array)
            .and_then(|point| Some((point.first()?.as_f64()?, point.get(1)?.as_f64()?)));
        let element = arguments.get("element_token").and_then(Value::as_str).and_then(|t| seen.centres.get(t).copied());
        let named = || {
            let scale = if desktop(arguments) { seen.desktop_scale } else { seen.window_scale }.unwrap_or(1.0);
            Some((arguments.get("x")?.as_f64()? / scale, arguments.get("y")?.as_f64()? / scale))
        };
        let keys = tool != "click";
        resolved
            .or(element)
            .or_else(named)
            .or(seen.last.filter(|_| keys))
            .filter(|(x, y)| x.is_finite() && y.is_finite())
    }
}

/// A click the display makes in place of the driver: on the process `pid`'s window, at `at` in the `frame` the
/// display's `c` line names (`w` the window's pixels, `o` the output's), of the element `element`, if it names one.
struct Fallback {
    pid: u64,
    frame: char,
    at: (f64, f64),
    element: Option<String>,
}

/// The tools that act at a place: a click, and keys into a focused element (at its point or element when named).
pub fn acts(tool: &str) -> bool {
    matches!(tool, "click" | "type_text" | "press_key" | "hotkey")
}

/// The tools whose arguments the admin needs beside their answer: those that act at a place, and the window reads it
/// learns snapshots from.
pub fn needs_arguments(tool: &str) -> bool {
    acts(tool) || tool == "get_window_state"
}

/// Whether the call names desktop pixels or the focused application rather than a window.
/// Every form the driver takes it in: `scope` (with a window too, it means desktop pixels sent to that window),
/// `coordinate_frame`, and a `target` of kind `desktop`.
fn desktop(arguments: &Value) -> bool {
    let is = |at: &str| arguments.pointer(at).and_then(Value::as_str) == Some("desktop");
    is("/scope") || is("/coordinate_frame") || is("/target/kind")
}

/// Whether the driver's injection client refuses `text` for its characters alone: one outside printable ASCII, tab and
/// newline, and no control character (which the display types for nobody: the driver's refusal says so).
fn beyond_driver(text: &str) -> bool {
    let typable = |c: char| c == '\n' || c == '\t' || (c.is_ascii() && !c.is_ascii_control());
    !text.chars().all(typable) && !text.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
}

fn failed(result: &Value) -> bool {
    result.get("isError").and_then(Value::as_bool) == Some(true)
}

/// A result's text items, together.
fn text(result: &Value) -> String {
    let items = result.get("content").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    items.iter().filter_map(|item| item.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n")
}

/// The centre of an element's frame.
fn centre(element: &Value) -> Option<(f64, f64)> {
    let frame = element.get("frame")?;
    let at = |key: &str| frame.get(key).and_then(Value::as_f64).filter(|v| v.is_finite());
    Some((at("x")? + at("w")? / 2.0, at("y")? + at("h")? / 2.0))
}

/// The driver's answer to a `type_text` of no text, counting `characters` instead.
fn typed(mut result: Value, characters: usize) -> Value {
    recount(&mut result, &format!("Typed {characters} character(s)"), characters);
    result
}

/// Says `typed` wherever `value` says the driver typed no character: its text items, and the structured result's
/// summary and count, whichever form the driver's projection gives them.
fn recount(value: &mut Value, typed: &str, characters: usize) {
    match value {
        Value::String(said) => *said = said.replacen("Typed 0 character(s)", typed, 1),
        Value::Array(items) => items.iter_mut().for_each(|item| recount(item, typed, characters)),
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                match item {
                    Value::Number(_) if key == "characters" => *item = json!(characters),
                    _ => recount(item, typed, characters),
                }
            }
        }
        _ => {}
    }
}

/// What the driver answers a command its compositor refused.
fn refused(why: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": format!("cua-compositor rejected the command: {why}")}],
        "isError": true,
        "structuredContent": {"code": "tool_invocation_failed"},
    })
}

/// The driver's answer to a pixel click, for one at `(x, y)` (of the element `element`, when it names one).
fn clicked(arguments: &Value, (x, y): (f64, f64), count: u32, element: Option<String>) -> Value {
    let mode = if arguments.get("delivery_mode").and_then(Value::as_str) == Some("foreground") {
        "foreground"
    } else {
        "background"
    };
    let (xi, yi) = (x as i32, y as i32);
    let what = match element {
        Some(token) => format!("element [{}]", token.rsplit(':').next().unwrap_or(&token)),
        None => format!("at ({x:.1}, {y:.1})"),
    };
    let text = format!(
        "Clicked {what} × {count} (delivery_mode={mode}, path={PATH}); not verified — confirm with a screenshot. \
         [window_point=({xi}, {yi}) screen_point=({xi}, {yi})]"
    );
    json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": {"path": PATH, "verified": false, "effect": "unverifiable", "delivery_mode": mode,
            "window_point": [xi, yi], "screen_point": [xi, yi]},
    })
}

/// A click's presses, as the display takes them (one to three).
fn count(arguments: &Value) -> u32 {
    arguments.get("count").and_then(Value::as_u64).map_or(1, |n| n.clamp(1, 3) as u32)
}

/// A click's button as an evdev code, none for one the display does not press.
fn button(arguments: &Value) -> Option<u32> {
    match arguments.get("button").and_then(Value::as_str) {
        None | Some("left") => Some(0x110),
        Some("right") => Some(0x111),
        Some("middle") => Some(0x112),
        Some(_) => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The driver's answer to a `type_text` it typed `n` characters of.
    /// As the pinned driver's projection gives it (measured end to end): its summary in the structured result too.
    pub(crate) fn typed_answer(n: usize) -> Value {
        let said = format!("Typed {n} character(s) (focus-free via cua-compositor).");
        json!({"content": [{"type": "text", "text": said}], "structuredContent": {"delivery": {"mode": "background"},
            "effect": "unverifiable", "route": "synthetic_events", "summary": said}})
    }

    /// An assist on a display that answers each line with `answer`, and the lines it was sent.
    fn on_display(
        name: &str,
        answer: &'static str,
    ) -> (Assist, std::sync::Arc<std::sync::Mutex<Vec<String>>>, crate::private::TestDir) {
        let dir = crate::private::test_dir(&format!("assist-{name}"));
        let lines = crate::inject::fake(dir.join("display.sock"), move |_| answer.to_owned());
        (Assist::new(dir.join("display.sock")), lines, dir)
    }

    /// The route `assist` gives a `type_text` of `text` with `arguments`, and the arguments the driver then gets.
    fn routed(assist: &Assist, mut arguments: Value) -> (Route, Value) {
        let route = assist.route("type_text", &mut arguments);
        (route, arguments)
    }

    #[tokio::test]
    async fn only_text_the_driver_cannot_type_into_a_known_process_s_window_goes_to_the_display() {
        let (assist, _lines, _dir) = on_display("routes", "ok");
        let (route, sent) = routed(&assist, json!({"pid": 7, "x": 10, "y": 20, "text": "é—中"}));
        assert_eq!(route, Route::Text { text: "é—中".into(), pid: 7 });
        assert_eq!(sent["text"], "", "the driver targets the call with no text");
        // By element: the process its snapshot was read of.
        let token = json!({"element_token": "s0000002a:3", "text": "é"});
        assert_eq!(routed(&assist, token.clone()).0, Route::Driver, "no snapshot of it relayed");
        let read = json!({"content": [], "structuredContent": {"snapshot_id": "s0000002a"}});
        let window = json!({"pid": 9, "window_id": 2});
        assist.answered(|| false, "get_window_state", Some(&window), Route::Driver, read).await.unwrap();
        assert_eq!(routed(&assist, token).0, Route::Text { text: "é".into(), pid: 9 });
        for (tool, mut call) in [
            ("type_text", json!({"pid": 7, "text": "plain ascii\twith\ttabs\n"})),
            // Into the focused application the driver types any text itself.
            ("type_text", json!({"scope": "desktop", "text": "é"})),
            // A control character: the driver says why it cannot type it.
            ("type_text", json!({"pid": 7, "text": "é\u{7}"})),
            // No process the text could be held to.
            ("type_text", json!({"window_id": 2, "text": "é"})),
            ("press_key", json!({"pid": 7, "key": "é"})),
        ] {
            let before = call.clone();
            assert_eq!(assist.route(tool, &mut call), Route::Driver, "{before}");
            assert_eq!(call, before);
        }
    }

    #[tokio::test]
    async fn typed_text_is_answered_as_the_driver_answers_text_and_only_while_the_agent_may_act() {
        let (assist, lines, _dir) = on_display("typed", "ok");
        let call = json!({"pid": 7, "x": 10, "y": 20, "text": "é—中"});
        let answer = assist.answered(
            || false,
            "type_text",
            Some(&call),
            Route::Text { text: "é—中".into(), pid: 7 },
            typed_answer(0),
        );
        assert_eq!(answer.await.unwrap(), typed_answer(3), "exactly the driver's answer for three characters");
        assert_eq!(*lines.lock().unwrap(), ["u root:7 c3a9e28094e4b8ad"], "into the process the driver targets");
        // A person holds control: the display is sent nothing, and the call is blocked as every call then is.
        let blocked = assist.answered(
            || true,
            "type_text",
            Some(&call),
            Route::Text { text: "é".into(), pid: 7 },
            typed_answer(0),
        );
        assert!(matches!(blocked.await, Err(CallError::Blocked)));
        assert_eq!(lines.lock().unwrap().len(), 1);
        // A driver that refused the target: its answer as it is, and nothing typed.
        let refusal = json!({"content": [{"type": "text", "text": "stale"}], "isError": true});
        let answer = assist.answered(
            || false,
            "type_text",
            Some(&call),
            Route::Text { text: "é".into(), pid: 7 },
            refusal.clone(),
        );
        assert_eq!(answer.await.unwrap(), refusal);
        assert_eq!(lines.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn control_taken_while_the_display_types_blocks_the_call() {
        let (assist, _lines, _dir) = on_display("held", "err control-held");
        let answer =
            assist.answered(|| false, "type_text", None, Route::Text { text: "é".into(), pid: 7 }, typed_answer(0));
        assert!(matches!(answer.await, Err(CallError::Blocked)));
        let (assist, _lines, _dir) = on_display("long", "err text-too-long");
        let answer = assist
            .answered(|| false, "type_text", None, Route::Text { text: "é".into(), pid: 7 }, typed_answer(0))
            .await;
        assert_eq!(answer.unwrap()["content"][0]["text"], "cua-compositor rejected the command: text-too-long");
    }

    /// A snapshot of two elements in the elements format, at a screenshot of `scale`.
    fn snapshot(scale: f64) -> Value {
        json!({"content": [{"type": "text", "text": "tree"}], "structuredContent": {"frame_scale": scale,
            "snapshot_id": "s0000002a", "elements": [
            {"element_index": 3, "element_token": "s0000002a:3", "role": "menu item", "label": "Mute",
                "actions": ["select", "showContextMenu"], "frame": {"x": 100, "y": 200, "w": 80, "h": 30}},
            {"element_index": 4, "element_token": "s0000002a:4", "role": "push button", "label": "Post"},
        ]}})
    }

    #[tokio::test]
    async fn the_agent_acts_where_its_element_its_point_or_its_last_place_is_and_the_agent_hears_nothing_of_it() {
        let (assist, _lines, _dir) = on_display("places", "ok");
        let mut places = assist.places();
        let read = assist.answered(|| false, "get_window_state", None, Route::Driver, snapshot(0.5)).await.unwrap();
        assert_eq!(read, snapshot(0.5), "the snapshot goes on as it is");
        let clicked = json!({"content": [{"type": "text", "text": "Clicked element [3] (pid 7)."}],
            "structuredContent": {"path": "ax", "verified": false, "effect": "unverifiable"}});
        let call = json!({"element_token": "s0000002a:3"});
        let answer = assist.answered(|| false, "click", Some(&call), Route::Driver, clicked.clone()).await;
        assert_eq!(answer.unwrap(), clicked, "the agent's answer is the driver's");
        assert_eq!(*places.borrow_and_update(), Some((140, 215)), "the centre of the element's frame");
        // Keys into the focused element: where the agent last acted.
        let pressed = json!({"content": [{"type": "text", "text": "Pressed enter."}]});
        assist.answered(|| false, "press_key", Some(&json!({"key": "enter"})), Route::Driver, pressed).await.unwrap();
        assert!(places.has_changed().unwrap());
        assert_eq!(*places.borrow_and_update(), Some((140, 215)));
        // Pixels read off the half-size screenshot, unless the driver says where they landed.
        let pixel = json!({"content": [{"type": "text", "text": "Clicked at (10.0, 20.0)"}]});
        let call = json!({"pid": 7, "window_id": 2, "x": 10, "y": 20});
        assist.answered(|| false, "hotkey", Some(&call), Route::Driver, pixel.clone()).await.unwrap();
        assert_eq!(*places.borrow_and_update(), Some((20, 40)));
        let resolved = json!({"content": [], "structuredContent": {"screen_point": [33, 44]}});
        assist.answered(|| false, "click", Some(&call), Route::Driver, resolved).await.unwrap();
        assert_eq!(*places.borrow_and_update(), Some((33, 44)));
        // An element of no known frame, or a failed call, acts nowhere.
        let call = json!({"element_token": "s0000002a:4"});
        assist.answered(|| false, "click", Some(&call), Route::Driver, clicked).await.unwrap();
        let failed = json!({"content": [], "isError": true});
        assist.answered(|| false, "press_key", Some(&json!({})), Route::Driver, failed).await.unwrap();
        assert!(!places.has_changed().unwrap());
    }

    #[tokio::test]
    async fn pixels_are_read_off_the_scale_of_the_screenshot_they_are_of_window_or_desktop() {
        let (assist, _lines, _dir) = on_display("scales", "ok");
        let mut places = assist.places();
        // A window downsized to half (its screenshot's pixels per window pixel), and a desktop to a quarter (the
        // driver says output pixels per screenshot pixel there).
        let window = json!({"content": [], "structuredContent": {"frame_scale": 0.5}});
        assist.answered(|| false, "get_window_state", Some(&json!({"pid": 7})), Route::Driver, window).await.unwrap();
        let desktop = json!({"content": [], "structuredContent": {"frame_scale": 4.0}});
        assist.answered(|| false, "get_desktop_state", Some(&json!({})), Route::Driver, desktop).await.unwrap();
        let pressed = json!({"content": [{"type": "text", "text": "Pressed."}]});
        let at = |call: Value| {
            let (assist, pressed) = (&assist, pressed.clone());
            async move { assist.answered(|| false, "hotkey", Some(&call), Route::Driver, pressed).await.unwrap() }
        };
        at(json!({"pid": 7, "window_id": 2, "x": 100, "y": 50, "keys": ["ctrl", "v"]})).await;
        assert_eq!(*places.borrow_and_update(), Some((200, 100)), "window pixels at half size");
        at(json!({"scope": "desktop", "x": 100, "y": 50, "keys": ["ctrl", "v"]})).await;
        assert_eq!(*places.borrow_and_update(), Some((400, 200)), "desktop pixels at a quarter");
        // Every form the driver takes desktop pixels in, and a window target's pixels.
        for (call, place) in [
            (json!({"coordinate_frame": "desktop", "pid": 7, "x": 10, "y": 10}), (40, 40)),
            (json!({"scope": "desktop", "pid": 7, "window_id": 2, "x": 10, "y": 10}), (40, 40)),
            (json!({"target": {"kind": "desktop", "display_id": "primary"}, "x": 10, "y": 10}), (40, 40)),
            (json!({"target": {"kind": "window", "pid": 7, "window_id": 2}, "x": 10, "y": 10}), (20, 20)),
            (json!({"scope": "window", "pid": 7, "window_id": 2, "x": 10, "y": 10}), (20, 20)),
        ] {
            at(call.clone()).await;
            assert_eq!(*places.borrow_and_update(), Some(place), "{call}");
        }
    }

    #[tokio::test]
    async fn a_click_the_driver_cannot_deliver_here_is_the_display_s_at_the_element_or_the_window_pixel() {
        let (assist, lines, _dir) = on_display("fallback", "ok");
        let call = json!({"element_token": "s0000002a:3"});
        let no_x11 = json!({"content": [{"type": "text", "text": "cannot inject input: X11 connection failed"}],
            "isError": true});
        let answer = assist.answered(|| false, "click", Some(&call), Route::Driver, no_x11.clone()).await;
        assert_eq!(answer.unwrap(), no_x11, "no snapshot of it relayed, so neither its frame nor its process");
        let window = json!({"pid": 7, "window_id": 2});
        assist.answered(|| false, "get_window_state", Some(&window), Route::Driver, snapshot(1.0)).await.unwrap();
        let no_x11 = json!({"content": [{"type": "text", "text":
            "AT-SPI element click failed: cannot inject input: X11 connection failed (DISPLAY=None): no display"}],
            "isError": true, "structuredContent": {"code": "tool_invocation_failed"}});
        let call = json!({"element_token": "s0000002a:3", "delivery_mode": "foreground"});
        let answer = assist.answered(|| false, "click", Some(&call), Route::Driver, no_x11.clone()).await.unwrap();
        assert_eq!(answer["isError"], Value::Null, "{answer}");
        assert_eq!(answer["structuredContent"]["path"], PATH);
        assert_eq!(answer["structuredContent"]["delivery_mode"], "foreground");
        assert!(answer["content"][0]["text"].as_str().unwrap().starts_with("Clicked element [3] × 1"), "{answer}");
        let no_shot = json!({"content": [{"type": "text", "text": "No screenshot frame for window 2"}], "isError": true,
            "structuredContent": {"code": NO_SCREENSHOT, "reason": "no_screenshot", "pid": 7, "window_id": 2}});
        let call = json!({"pid": 7, "window_id": 2, "x": 10.5, "y": 20, "count": 2, "button": "right"});
        let answer = assist.answered(|| false, "click", Some(&call), Route::Driver, no_shot.clone()).await.unwrap();
        assert_eq!(answer["structuredContent"]["screen_point"], json!([10, 20]));
        assert_eq!(
            *lines.lock().unwrap(),
            ["c root:7 o 140 215 1 272", "c root:7 w 10.5 20 2 273"],
            "on the process alone"
        );
        // A window the driver never read, read by another session, or another than the one named keeps its refusal.
        for (reason, window) in [("window_never_read", 2), ("other_session", 2), ("no_screenshot", 3)] {
            let refusal = json!({"content": [], "isError": true,
                "structuredContent": {"code": NO_SCREENSHOT, "reason": reason, "pid": 7, "window_id": window}});
            let answer = assist.answered(|| false, "click", Some(&call), Route::Driver, refusal.clone()).await;
            assert_eq!(answer.unwrap(), refusal, "{reason} {window}");
        }
        // An element of no known frame, a modified click, desktop pixels, and a person in control are left as the
        // driver answered, or blocked.
        let unknown = json!({"element_token": "s0000002a:9"});
        assert_eq!(
            assist.answered(|| false, "click", Some(&unknown), Route::Driver, no_x11.clone()).await.unwrap(),
            no_x11
        );
        let modified = json!({"element_token": "s0000002a:3", "modifier": ["ctrl"]});
        assert_eq!(
            assist.answered(|| false, "click", Some(&modified), Route::Driver, no_x11.clone()).await.unwrap(),
            no_x11
        );
        let desktop = json!({"scope": "desktop", "x": 1, "y": 2});
        assert_eq!(
            assist.answered(|| false, "click", Some(&desktop), Route::Driver, no_shot.clone()).await.unwrap(),
            no_shot
        );
        let call = json!({"element_token": "s0000002a:3"});
        assert!(matches!(
            assist.answered(|| true, "click", Some(&call), Route::Driver, no_x11).await,
            Err(CallError::Blocked)
        ));
        assert_eq!(lines.lock().unwrap().len(), 2);
        // The display refuses a click whose target is not the window shown, or what is under the point another's.
        let (assist, _lines, _dir) = on_display("elsewhere", "err other-surface-at-point");
        let answer = assist
            .answered(
                || false,
                "click",
                Some(&json!({"pid": 7, "window_id": 2, "x": 1, "y": 1})),
                Route::Driver,
                no_shot.clone(),
            )
            .await
            .unwrap();
        assert_eq!(answer["isError"], true);
        assert_eq!(answer["content"][0]["text"], "cua-compositor rejected the command: other-surface-at-point");
    }
}
