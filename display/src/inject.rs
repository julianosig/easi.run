//! The `cua-inject v1` control socket, the injection route the pinned Cua Driver uses with its own nested compositor.
//!
//! One command per line, each answered by exactly one line, sent only after the effect has been applied to the seat
//! and flushed to the browser. The socket lives in the privileged directory and accepts only the agent's uid.
//!
//! Process tokens (`pid:N`, `root:N`, `q N`, `g N`, `f N`) come from the agent's PID namespace, which the compositor
//! does not share, so it cannot read the browser's credentials in that namespace. Its identity check is instead the
//! socket: every toplevel belongs to a client of the browser socket, whose uid the kernel attested. A token resolves
//! only while exactly one browser toplevel is mapped, and the first PID the agent names is bound for the life of
//! that toplevel; a later different PID is refused as unknown until another toplevel (a relaunched browser) shows.

use display_protocol::keys;
use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::rc::Rc;

use calloop::generic::Generic;
use calloop::{Interest, Mode, PostAction};
use smithay::input::keyboard::xkb;

use crate::input::{self, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT};
use crate::limits;
use crate::state::{Data, State};

const HELLO: &str = "cua-inject v1";
const MAX_LINE_BUFFER: usize = 16 * 1024;

/// US-layout ASCII -> (evdev key code, needs shift), from the same keymap the seat uses, and that keymap's text.
pub struct CharTable {
    table: [Option<(u32, bool)>; 128],
    pub keymap: String,
}

impl CharTable {
    pub fn new() -> CharTable {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(&context, "", "", "", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS)
            .expect("the default keymap compiles");
        let mut table = [None; 128];
        for code in 9..256u32 {
            for level in 0..2 {
                let syms = keymap.key_get_syms_by_level(code.into(), 0, level);
                if let [sym] = syms {
                    let cp = xkb::keysym_to_utf32(*sym);
                    if cp > 0 && cp < 128 && table[cp as usize].is_none() {
                        table[cp as usize] = Some((code - 8, level == 1));
                    }
                }
            }
        }
        table[b'\n' as usize] = Some((keys::ENTER.into(), false));
        table[b'\t' as usize] = Some((keys::TAB.into(), false));
        CharTable { table, keymap: keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1) }
    }

    pub fn get(&self, c: u8) -> Option<(u32, bool)> {
        self.table.get(c as usize).copied().flatten()
    }
}

/// One live injection connection, counted until its source goes.
struct Slot(Rc<Cell<usize>>);

impl Slot {
    fn new(live: &Rc<Cell<usize>>) -> Slot {
        live.set(live.get() + 1);
        Slot(live.clone())
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

/// Serves the agent's connections, at most [`limits::MAX_INJECT_CONNECTIONS`] at once (a further one is closed), so
/// the agent cannot take every descriptor the compositor has.
pub fn listen(handle: &calloop::LoopHandle<'static, Data>, listener: UnixListener, agent_uid: u32) {
    listener.set_nonblocking(true).expect("non-blocking listener");
    let live = Rc::new(Cell::new(0));
    handle
        .insert_source(Generic::new(listener, Interest::READ, Mode::Level), move |_, listener, data| {
            while let Ok((stream, _)) = listener.accept() {
                let peer = crate::peer_uid(&stream);
                if peer != Some(agent_uid) {
                    tracing::warn!(?peer, "refused an injection connection from another uid");
                    continue;
                }
                if live.get() >= limits::MAX_INJECT_CONNECTIONS {
                    tracing::warn!("refused an injection connection past the limit");
                    continue;
                }
                if stream.set_nonblocking(true).is_ok() {
                    connection(&data.state.handle, stream, Slot::new(&live));
                }
            }
            Ok(PostAction::Continue)
        })
        .expect("inject listener source");
}

/// One injection connection: the lines it sent not answered yet, and the answer that waits for typed text.
struct Connection {
    /// The connection's socket, a second descriptor of the one its source reads, for the answers.
    stream: UnixStream,
    buffer: Vec<u8>,
    hello: bool,
    /// Whether the connection waits for text being typed, with the answer it then sends, if any.
    waiting: bool,
    answer: Option<String>,
    /// The slot goes with the connection.
    _slot: Slot,
}

fn connection(handle: &calloop::LoopHandle<'static, Data>, stream: UnixStream, slot: Slot) {
    let Ok(answers) = stream.try_clone() else { return };
    let conn = Rc::new(RefCell::new(Connection {
        stream: answers,
        buffer: Vec::new(),
        hello: false,
        waiting: false,
        answer: None,
        _slot: slot,
    }));
    let _ = handle.insert_source(Generic::new(stream, Interest::READ, Mode::Level), move |_, stream, data| {
        let mut chunk = [0u8; 4096];
        let n = match (&**stream).read(&mut chunk) {
            Ok(0) => return Ok(PostAction::Remove),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(PostAction::Continue),
            Err(_) => return Ok(PostAction::Remove),
        };
        let mut c = conn.borrow_mut();
        c.buffer.extend_from_slice(&chunk[..n]);
        if c.buffer.len() > MAX_LINE_BUFFER {
            return Ok(PostAction::Remove);
        }
        drop(c);
        Ok(if serve(&conn, &mut data.state) { PostAction::Continue } else { PostAction::Remove })
    });
}

/// Answers the complete lines buffered, in order. While text is being typed over several turns, the next line waits
/// for it, and so does the answer of the line that typed it: each answer still follows its effect, and every effect
/// lands after the text before it. Returns whether the connection stays.
fn serve(conn: &Rc<RefCell<Connection>>, state: &mut State) -> bool {
    loop {
        let mut c = conn.borrow_mut();
        if c.waiting {
            return true;
        }
        let Some(end) = c.buffer.iter().position(|&b| b == b'\n') else { return true };
        if c.hello && input::typing(state) {
            drop(c);
            wait(conn, state, None);
            return true;
        }
        let line: Vec<u8> = c.buffer.drain(..=end).collect();
        let line = String::from_utf8_lossy(&line[..line.len() - 1]).trim_end_matches('\r').to_owned();
        let reply = if c.hello {
            let reply = handle_line(state, &line);
            if input::typing(state) {
                drop(c);
                wait(conn, state, Some(reply));
                return true;
            }
            // Deliver the injected events before acknowledging them.
            let _ = state.dh.flush_clients();
            reply
        } else if line == HELLO {
            c.hello = true;
            HELLO.to_owned()
        } else {
            let _ = (&c.stream).write_all(b"err unsupported-version\n");
            return false;
        };
        if (&c.stream).write_all(format!("{reply}\n").as_bytes()).is_err() {
            return false;
        }
    }
}

/// Holds the connection, with `answer` if it has one, until the text being typed is in.
fn wait(conn: &Rc<RefCell<Connection>>, state: &mut State, answer: Option<String>) {
    {
        let mut c = conn.borrow_mut();
        c.waiting = true;
        c.answer = answer;
    }
    let conn = conn.clone();
    input::when_typed(state, move |state, typed| resume(&conn, state, typed));
}

/// Sends the answer that waited for typed text (refused when a lease change dropped the text) and serves the lines
/// behind it. A connection that cannot be answered is shut down, so its source sees it end.
fn resume(conn: &Rc<RefCell<Connection>>, state: &mut State, typed: bool) {
    let answer = {
        let mut c = conn.borrow_mut();
        c.waiting = false;
        c.answer.take()
    };
    let sent = match answer {
        Some(answer) => {
            let answer = if typed { answer } else { err("control-held") };
            let _ = state.dh.flush_clients();
            (&conn.borrow().stream).write_all(format!("{answer}\n").as_bytes()).is_ok()
        }
        None => true,
    };
    if !sent || !serve(conn, state) {
        let _ = conn.borrow().stream.shutdown(std::net::Shutdown::Both);
    }
}

fn err(token: &str) -> String {
    format!("err {token}")
}

fn parse_pid(text: &str) -> Option<u32> {
    text.parse::<u32>().ok().filter(|&pid| pid > 0)
}

/// Resolves a PID the agent names against the browser toplevel, binding the first one it names. A PID names no window
/// (the driver drops the window it means), so input targeted by PID is refused while more than one window is shown:
/// it could reach another window than the agent's. Desktop-scope input still goes to the window shown.
fn resolve_pid(state: &mut State, pid: u32) -> Result<(), &'static str> {
    let mapped = state.toplevels.iter().filter(|t| state.scene.is_mapped(t.surface.wl_surface())).count();
    if mapped > 1 {
        return Err("ambiguous-pid");
    }
    let Some(top) = state.browser_toplevel() else { return Err("unknown-pid") };
    let surface = top.surface.wl_surface().clone();
    bind(&mut state.inject_binding, pid, surface)
}

/// Binds `pid` to the toplevel `surface` unless another PID is already bound to that same toplevel. A binding to a
/// toplevel no longer shown gives way: a relaunched browser comes back as a new toplevel under a new PID.
fn bind<S: PartialEq>(binding: &mut Option<(u32, S)>, pid: u32, surface: S) -> Result<(), &'static str> {
    match binding {
        Some((bound, toplevel)) if *toplevel == surface && *bound != pid => Err("unknown-pid"),
        _ => {
            *binding = Some((pid, surface));
            Ok(())
        }
    }
}

fn resolve_target(state: &mut State, target: &str) -> Result<(), String> {
    if let Some(pid) = target.strip_prefix("root:") {
        let pid = parse_pid(pid).ok_or_else(|| err("bad-root-pid"))?;
        return resolve_pid(state, pid).map_err(|e| err(&e.replace("pid", "root-pid")));
    }
    if let Some(pid) = target.strip_prefix("pid:") {
        let pid = parse_pid(pid).ok_or_else(|| err("bad-pid"))?;
        return resolve_pid(state, pid).map_err(err);
    }
    let mapped: Vec<&str> = state
        .toplevels
        .iter()
        .filter(|t| state.scene.is_mapped(t.surface.wl_surface()))
        .map(|t| t.app_id.as_str())
        .collect();
    resolve_app_id(&mapped, target).map_err(err)
}

/// Resolves an app id against the mapped toplevels' app ids, bottom to top. Input goes to the top one, the one shown
/// (see [`State::browser_toplevel`]), so an app id that names only a window below it is refused as ambiguous, as a PID
/// is while several windows show: the input would reach another window than the one named.
fn resolve_app_id(mapped: &[&str], target: &str) -> Result<(), &'static str> {
    match mapped.iter().filter(|&&app_id| app_id == target).count() {
        0 => Err("unknown-app-id"),
        1 if mapped.last() == Some(&target) => Ok(()),
        _ => Err("ambiguous-app-id"),
    }
}

/// Window-local coordinates are relative to the toplevel's root surface, which this output keeps at the origin.
fn window_to_output(state: &State, x: f64, y: f64) -> (f64, f64) {
    let origin = state.browser_toplevel().map(|t| crate::shell::toplevel_origin(t).0).unwrap_or((0, 0));
    (origin.0 as f64 + x, origin.1 as f64 + y)
}

fn named_key(name: &str) -> Option<u32> {
    let name = name.to_ascii_lowercase();
    let code = match name.as_str() {
        "enter" | "return" => keys::ENTER,
        "tab" => keys::TAB,
        "escape" | "esc" => keys::ESC,
        "backspace" => keys::BACKSPACE,
        "space" => keys::SPACE,
        "up" => keys::UP,
        "down" => keys::DOWN,
        "left" => keys::LEFT,
        "right" => keys::RIGHT,
        "f11" => keys::F11,
        "f12" => keys::F12,
        f if f.starts_with('f') => {
            let n: u16 = f[1..].parse().ok()?;
            if !(1..=10).contains(&n) {
                return None;
            }
            keys::F1 + n - 1
        }
        _ => return None,
    };
    Some(code.into())
}

fn tap(state: &mut State, code: u32) {
    input::key(state, code, true);
    input::key(state, code, false);
}

fn handle_line(state: &mut State, line: &str) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(&command) = words.first() else { return err("empty") };
    // Reads first: they never change the browser and are allowed while a person holds control.
    match (command, words.len()) {
        ("q", 2) => {
            let Some(pid) = parse_pid(words[1]) else { return err("bad-args") };
            return match resolve_pid(state, pid) {
                Ok(()) => format!("state {pid} foreground"),
                Err(_) => "state 0 not_found".to_owned(),
            };
        }
        ("g", 2) => {
            let Some(pid) = parse_pid(words[1]) else { return err("bad-args") };
            if let Err(e) = resolve_pid(state, pid) {
                return err(if e == "unknown-pid" { "target-not-found" } else { e });
            }
            let Some(top) = state.browser_toplevel() else { return err("target-not-found") };
            let ((sx, sy), (gx, gy)) = crate::shell::toplevel_origin(top);
            return format!("geometry {} {} {} {}", sx.saturating_sub(gx), sy.saturating_sub(gy), sx, sy);
        }
        _ => {}
    }
    if !input::agent_may_act(state) {
        return err("control-held");
    }
    match command {
        "f" if words.len() == 2 => {
            let Some(pid) = parse_pid(words[1]) else { return err("bad-args") };
            match resolve_pid(state, pid) {
                Ok(()) => "ok".to_owned(),
                Err(e) => err(e),
            }
        }
        "d" if words.len() == 5 => {
            let (Ok(x), Ok(y), Ok(count), Ok(button)) =
                (words[1].parse::<f64>(), words[2].parse::<f64>(), words[3].parse::<u32>(), words[4].parse::<u32>())
            else {
                return err("bad-args");
            };
            if ![BTN_LEFT, BTN_RIGHT, BTN_MIDDLE].contains(&button) || count > 3 || !finite(x, y) {
                return err("bad-args");
            }
            if input::surface_under(state, x, y).is_none() {
                return err("no-surface-at-point");
            }
            input::pointer_move(state, x, y);
            for _ in 0..count.max(1) {
                input::pointer_button(state, button, true);
                input::pointer_button(state, button, false);
            }
            "ok".to_owned()
        }
        "m" if words.len() == 5 => {
            if let Err(e) = resolve_target(state, words[1]) {
                return e;
            }
            let (Ok(index), Ok(x), Ok(y)) = (words[2].parse::<i32>(), words[3].parse::<f64>(), words[4].parse::<f64>())
            else {
                return err("bad-args");
            };
            if !finite(x, y) {
                return err("bad-args");
            }
            if index != 0 {
                return err("no-pointer-resource");
            }
            let (x, y) = window_to_output(state, x, y);
            input::pointer_move(state, x, y);
            "ok".to_owned()
        }
        "b" if words.len() == 5 => {
            if let Err(e) = resolve_target(state, words[1]) {
                return e;
            }
            let (Ok(index), Ok(button), Ok(pressed)) =
                (words[2].parse::<i32>(), words[3].parse::<u32>(), words[4].parse::<u32>())
            else {
                return err("bad-args");
            };
            if index != 0 || ![BTN_LEFT, BTN_RIGHT, BTN_MIDDLE].contains(&button) {
                return err("no-pointer-resource");
            }
            input::pointer_button(state, button, pressed != 0);
            "ok".to_owned()
        }
        "a" if words.len() == 5 => {
            if let Err(e) = resolve_target(state, words[1]) {
                return e;
            }
            let (Ok(index), Ok(axis), Ok(value)) =
                (words[2].parse::<i32>(), words[3].parse::<u32>(), words[4].parse::<f64>())
            else {
                return err("bad-args");
            };
            if index != 0 || axis > 1 || !value.is_finite() || value.abs() > 10_000.0 {
                return err("bad-args");
            }
            let (dx, dy) = if axis == 1 { (value, 0.0) } else { (0.0, value) };
            input::pointer_notch(state, dx, dy);
            "ok".to_owned()
        }
        "t" if words.len() == 3 => {
            if let Err(e) = resolve_target(state, words[1]) {
                return e;
            }
            let Some(bytes) = decode_hex(words[2]) else { return err("bad-args") };
            let keys: Option<Vec<(u32, bool)>> = bytes.iter().map(|&b| state.chars.get(b)).collect();
            let Some(keys) = keys else { return err("no-keyboard-resource") };
            input::type_ascii_text(state, keys);
            "ok".to_owned()
        }
        "k" if words.len() == 3 => {
            if let Err(e) = resolve_target(state, words[1]) {
                return e;
            }
            let Some(code) = named_key(words[2]) else { return err("unknown-key") };
            tap(state, code);
            "ok".to_owned()
        }
        "h" if words.len() == 4 => {
            if let Err(e) = resolve_target(state, words[1]) {
                return e;
            }
            let mut modifiers = Vec::new();
            for m in words[2].split(',') {
                let modifier = match m.to_ascii_lowercase().as_str() {
                    "ctrl" | "control" => keys::LEFTCTRL,
                    "shift" => keys::LEFTSHIFT,
                    "alt" | "option" => keys::LEFTALT,
                    "meta" | "super" | "win" | "cmd" => keys::LEFTMETA,
                    _ => return err("unknown-hotkey"),
                };
                modifiers.push(u32::from(modifier));
            }
            let key = named_key(words[3]).or_else(|| match words[3].as_bytes() {
                [c] => state.chars.get(c.to_ascii_lowercase()).map(|(code, _)| code),
                _ => None,
            });
            let Some(key) = key else { return err("unknown-hotkey") };
            for &m in &modifiers {
                input::key(state, m, true);
            }
            tap(state, key);
            for &m in modifiers.iter().rev() {
                input::key(state, m, false);
            }
            "ok".to_owned()
        }
        _ => err("unknown-command"),
    }
}

/// Whether a point is one: `nan` and `inf` parse as numbers, but no pointer can be at them.
fn finite(x: f64, y: f64) -> bool {
    x.is_finite() && y.is_finite()
}

/// Decodes hex digits by bytes, so a request with non-ASCII text (which slicing a `str` would split mid-character)
/// is refused rather than panicking the compositor.
fn decode_hex(text: &str) -> Option<Vec<u8>> {
    let digits = text.as_bytes();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    let value = |d: u8| char::from(d).to_digit(16);
    digits.chunks(2).map(|pair| Some((value(pair[0])? * 16 + value(pair[1])?) as u8)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_char_table_covers_printable_ascii() {
        let table = CharTable::new();
        for c in 0x20u8..0x7f {
            assert!(table.get(c).is_some(), "no key for {:?}", c as char);
        }
        assert_eq!(table.get(b'a'), Some((30, false)));
        assert_eq!(table.get(b'A'), Some((30, true)));
        assert_eq!(table.get(b'\n'), Some((keys::ENTER.into(), false)));
        assert_eq!(table.get(0x80), None);
    }

    #[test]
    fn a_slot_counts_its_connection_until_it_goes() {
        let live = Rc::new(Cell::new(0));
        let first = Slot::new(&live);
        let second = Slot::new(&live);
        assert_eq!(live.get(), 2);
        drop(first);
        drop(second);
        assert_eq!(live.get(), 0);
    }

    #[test]
    fn a_pid_stays_bound_to_its_toplevel_and_a_new_toplevel_takes_a_new_pid() {
        let mut binding = None;
        assert_eq!(bind(&mut binding, 10, "first"), Ok(()));
        assert_eq!(bind(&mut binding, 10, "first"), Ok(()));
        assert_eq!(bind(&mut binding, 11, "first"), Err("unknown-pid"));
        // The browser relaunched: its new toplevel binds the new PID, and the old one is refused from then on.
        assert_eq!(bind(&mut binding, 11, "second"), Ok(()));
        assert_eq!(bind(&mut binding, 10, "second"), Err("unknown-pid"));
    }

    #[test]
    fn an_app_id_resolves_only_to_the_window_shown() {
        assert_eq!(resolve_app_id(&["chromium"], "chromium"), Ok(()));
        assert_eq!(resolve_app_id(&["other", "chromium"], "chromium"), Ok(()));
        assert_eq!(resolve_app_id(&[], "chromium"), Err("unknown-app-id"));
        assert_eq!(resolve_app_id(&["other"], "chromium"), Err("unknown-app-id"));
        // The named window is below another: input would reach the one on top.
        assert_eq!(resolve_app_id(&["chromium", "other"], "chromium"), Err("ambiguous-app-id"));
        assert_eq!(resolve_app_id(&["chromium", "chromium"], "chromium"), Err("ambiguous-app-id"));
    }

    #[test]
    fn named_keys_match_the_driver_whitelist() {
        for name in ["enter", "Return", "tab", "esc", "backspace", "space", "up", "down", "left", "right"] {
            assert!(named_key(name).is_some(), "{name}");
        }
        assert_eq!(named_key("f1"), Some(keys::F1.into()));
        assert_eq!(named_key("F10"), Some((keys::F1 + 9).into()));
        assert_eq!(named_key("f12"), Some(keys::F12.into()));
        assert_eq!(named_key("f13"), None);
        assert_eq!(named_key("delete"), None);
        assert_eq!(decode_hex("6869"), Some(b"hi".to_vec()));
        assert_eq!(decode_hex("6"), None);
        // Non-ASCII of an even byte length is refused whole, never cut inside a character.
        assert_eq!(decode_hex("aéb"), None);
        assert_eq!(decode_hex("zz"), None);
    }

    #[test]
    fn points_that_parse_but_are_no_number_are_refused() {
        for word in ["nan", "NaN", "inf", "-inf", "infinity"] {
            let value: f64 = word.parse().unwrap();
            assert!(!finite(value, 1.0) && !finite(1.0, value), "{word}");
        }
        assert!(finite(-5.0, 1e6));
    }
}
