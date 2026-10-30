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
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use wayland_server::Resource;

use crate::input::{self, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT};
use crate::limits;
use crate::state::{Data, State};

const HELLO: &str = "cua-inject v1";

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
    let live = Rc::new(Cell::new(0));
    crate::listen_unix(handle, listener, agent_uid, "an injection", move |stream, data| {
        if live.get() >= limits::MAX_INJECT_CONNECTIONS {
            tracing::warn!("refused an injection connection past the limit");
        } else if stream.set_nonblocking(true).is_ok() {
            connection(&data.state.handle, stream, Slot::new(&live));
        }
    });
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
        if c.buffer.len() > limits::MAX_INJECT_LINE {
            // Answered, so the agent's tool call fails with a reason instead of an end of file. While a line waits for
            // typed text its own answer is still owed, and this one would be read as it: that connection only ends.
            if !c.waiting {
                let _ = (&c.stream).write_all(b"err line-too-long\n");
            }
            return Ok(PostAction::Remove);
        }
        // Without a new line end there is no new line to serve: a long line is looked through once, when it is whole.
        if !chunk[..n].contains(&b'\n') {
            return Ok(PostAction::Continue);
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

/// Sends the answer that waited for typed text (refused with why when the text was dropped) and serves the lines behind
/// it. A connection that cannot be answered is shut down, so its source sees it end.
fn resume(conn: &Rc<RefCell<Connection>>, state: &mut State, typed: Result<(), &'static str>) {
    let answer = {
        let mut c = conn.borrow_mut();
        c.waiting = false;
        c.answer.take()
    };
    let sent = match answer {
        Some(answer) => {
            let answer = typed.map_or_else(err, |()| answer);
            let _ = state.dh.flush_clients();
            (&conn.borrow().stream).write_all(format!("{answer}\n").as_bytes()).is_ok()
        }
        None => true,
    };
    if !sent || !serve(conn, state) {
        let _ = conn.borrow().stream.shutdown(std::net::Shutdown::Both);
    }
}

/// Serves one line the admin sent on the agent's behalf (see `frame::Link::Inject`) as this socket serves a
/// connection's: after the text still being typed, under the same rules, and answered once its own text is in.
pub fn serve_link(state: &mut State, line: String, answer: impl FnOnce(&mut State, String) + 'static) {
    if input::typing(state) {
        input::when_typed(state, move |state, _| serve_link(state, line, answer));
        return;
    }
    let reply = handle_line(state, &line);
    if input::typing(state) {
        input::when_typed(state, move |state, typed| answer(state, typed.map_or_else(err, |()| reply)));
        return;
    }
    let _ = state.dh.flush_clients();
    answer(state, reply);
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

/// The toplevel shown, which a line's text is for once its target resolved to it, while the keyboard's focus is its
/// client's: each slice of the text goes only while it still is (see `input::target_kept`).
fn keyboard_target(state: &State) -> Option<WlSurface> {
    let toplevel = state.browser_toplevel()?.surface.wl_surface().clone();
    let focus = state.seat.get_keyboard().and_then(|keyboard| keyboard.current_focus()).and_then(|f| f.client());
    (focus.is_some() && focus == toplevel.client()).then_some(toplevel)
}

/// The target an empty `t` resolved to, for the one `u` after it: the PID it named, and the toplevel and client that
/// PID resolved to then. Kept apart from the PID's binding, which any later lookup of the PID (`q`, `g`, another `t`)
/// moves to whatever toplevel shows by then; only an empty `t` sets it, any other `t` and a lease change clear it, and
/// a `u` consumes it.
pub type Pending = (u32, (WlSurface, wayland_server::Client));

/// The toplevel shown, with its client, alone: what a PID resolves to (see [`resolve_pid`]).
fn shown_alone(state: &State) -> Option<(WlSurface, wayland_server::Client)> {
    let mapped = state.toplevels.iter().filter(|t| state.scene.is_mapped(t.surface.wl_surface())).count();
    let toplevel = state.browser_toplevel().filter(|_| mapped == 1)?.surface.wl_surface().clone();
    let client = toplevel.client()?;
    Some((toplevel, client))
}

/// Checks the target of a `u` line, and consumes it: the PID of the empty `t` just before it, whose toplevel and client
/// must still be the ones shown, alone. It binds nothing, so a window that replaced that toplevel in between is
/// refused, whatever was bound since.
fn held_target(state: &mut State, target: &str) -> Result<(), &'static str> {
    let pid = target.strip_prefix("root:").or_else(|| target.strip_prefix("pid:")).and_then(parse_pid);
    let Some(pid) = pid else { return Err("bad-args") };
    let shown = shown_alone(state);
    claim(&mut state.pending_text, pid, shown.as_ref())
}

/// Takes `pending` for a `u` of `pid` while `shown` is what it names; once, whether it holds or not.
fn claim<S: PartialEq>(pending: &mut Option<(u32, S)>, pid: u32, shown: Option<&S>) -> Result<(), &'static str> {
    match pending.take() {
        Some((named, target)) if named == pid && shown == Some(&target) => Ok(()),
        _ => Err("target-changed"),
    }
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
    // Every command but `f` and `d` names its target first (`c` too, and its click lands only on that target). `u`
    // names one already bound (see `held`). A `t` of no text only names it: the driver's text comes
    // that way when the admin types it with `u` (see `u`), after the driver focused its target.
    // Whatever a `t` names, refused or not, no target an empty `t` named before it waits for a `u` any more.
    if command == "t" {
        state.pending_text = None;
    }
    let targeted =
        matches!((command, words.len()), ("m" | "b" | "a", 5) | ("t", 2 | 3) | ("k", 3) | ("h", 4) | ("c", 7));
    if targeted && let Err(e) = resolve_target(state, words[1]) {
        return e;
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
            click(state, x, y, count, button);
            "ok".to_owned()
        }
        // A click on its target alone, at a point of its window (`w`, as `m` takes one) or of the output (`o`, as `d`
        // does): refused unless what is under the point is the target's own. What the admin sends for a click the
        // driver cannot deliver here, so it lands on the window the agent named or nowhere.
        "c" if words.len() == 7 => {
            let (Ok(x), Ok(y), Ok(count), Ok(button)) =
                (words[3].parse::<f64>(), words[4].parse::<f64>(), words[5].parse::<u32>(), words[6].parse::<u32>())
            else {
                return err("bad-args");
            };
            if ![BTN_LEFT, BTN_RIGHT, BTN_MIDDLE].contains(&button) || count > 3 || !finite(x, y) {
                return err("bad-args");
            }
            let (x, y) = match words[2] {
                "w" => window_to_output(state, x, y),
                "o" => (x, y),
                _ => return err("bad-args"),
            };
            let Some(target) = state.browser_toplevel().and_then(|t| t.surface.wl_surface().client()) else {
                return err("target-not-found");
            };
            let under = input::surface_under(state, x, y).and_then(|(surface, _)| surface.client());
            if let Err(why) = landing(under.as_ref(), &target) {
                return err(why);
            }
            // What decides where a press goes: a grab or a held button would deliver it to the surface they hold, and
            // a drag of files keeps the pointer where it is.
            let grabbed = state.seat.get_pointer().is_some_and(|pointer| pointer.is_grabbed());
            if let Err(why) = free(grabbed, !state.buttons.is_empty(), crate::drop::dragging_files(state)) {
                return err(why);
            }
            input::pointer_move(state, x, y);
            let focus = state.seat.get_pointer().and_then(|pointer| pointer.current_focus()).and_then(|f| f.client());
            if let Err(why) = landing(focus.as_ref(), &target) {
                return err(why);
            }
            click(state, x, y, count, button);
            "ok".to_owned()
        }
        "m" if words.len() == 5 => {
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
        "t" if words.len() == 2 => {
            let pid = words[1].strip_prefix("root:").or_else(|| words[1].strip_prefix("pid:")).and_then(parse_pid);
            state.pending_text = pid.zip(shown_alone(state));
            "ok".to_owned()
        }
        "t" if words.len() == 3 => {
            let Some(bytes) = decode_hex(words[2]) else { return err("bad-args") };
            if bytes.len() > limits::MAX_INJECT_TEXT {
                return err("text-too-long");
            }
            let keys: Option<Vec<(u32, bool)>> = bytes.iter().map(|&b| state.chars.get(b)).collect();
            let Some(keys) = keys else { return err("no-keyboard-resource") };
            let Some(target) = keyboard_target(state) else { return err("target-changed") };
            input::type_ascii_text(state, keys, target);
            "ok".to_owned()
        }
        // Text of any characters, as UTF-8, into its target as `t` types: what the pinned driver cannot type with `t`
        // (printable ASCII, tab and newline only), sent by the admin on the agent's behalf. It types as a person's text
        // does, a character the keymap lacks with the spare key, and at most as much as a person's text message
        // carries: every character outside the keymap switches it, so text of ever different characters takes
        // milliseconds each.
        "u" if words.len() == 3 => match held_target(state, words[1]).and_then(|()| composed(words[2])) {
            Ok(text) => {
                let Some(target) = keyboard_target(state) else { return err("target-changed") };
                input::type_text_into(state, &text, target);
                "ok".to_owned()
            }
            Err(why) => err(why),
        },
        "k" if words.len() == 3 => {
            let Some(code) = named_key(words[2]) else { return err("unknown-key") };
            input::tap(state, code);
            "ok".to_owned()
        }
        "h" if words.len() == 4 => {
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
            input::tap(state, key);
            for &m in modifiers.iter().rev() {
                input::key(state, m, false);
            }
            "ok".to_owned()
        }
        _ => err("unknown-command"),
    }
}

/// Whether a click lands on the target's client: the surface under the point, or the pointer's focus once it is there,
/// is its own.
fn landing<C: PartialEq>(surface: Option<&C>, target: &C) -> Result<(), &'static str> {
    match surface {
        None => Err("no-surface-at-point"),
        Some(surface) if surface == target => Ok(()),
        Some(_) => Err("other-surface-at-point"),
    }
}

/// Whether the pointer is free to deliver a click where it is moved: no grab holds it, no button is held, and no drag
/// of files keeps it.
fn free(grabbed: bool, held: bool, dragging: bool) -> Result<(), &'static str> {
    if grabbed || held || dragging { Err("pointer-busy") } else { Ok(()) }
}

/// Moves the pointer to the output point (x, y) and clicks `button` there `count` times (once for none).
fn click(state: &mut State, x: f64, y: f64, count: u32, button: u32) {
    input::pointer_move(state, x, y);
    for _ in 0..count.max(1) {
        input::pointer_button(state, button, true);
        input::pointer_button(state, button, false);
    }
}

/// Whether a point is one: `nan` and `inf` parse as numbers, but no pointer can be at them.
fn finite(x: f64, y: f64) -> bool {
    x.is_finite() && y.is_finite()
}

/// The text of a `u` line: UTF-8 in hex digits, at most [`display_protocol::MAX_TEXT`] bytes, and no control character
/// but newline and tab (no key types one).
fn composed(hex: &str) -> Result<String, &'static str> {
    let text = decode_hex(hex).and_then(|bytes| String::from_utf8(bytes).ok()).ok_or("bad-args")?;
    if text.len() > display_protocol::MAX_TEXT {
        return Err("text-too-long");
    }
    if text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err("no-keyboard-resource");
    }
    Ok(text)
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
    fn a_u_line_types_only_into_what_the_empty_t_before_it_named_once_whatever_was_bound_since() {
        // `t root:7` binds window A and leaves it for the `u` after it; then B replaces A, and a `g 7` from another
        // connection binds 7 to B.
        let mut binding = None;
        assert_eq!(bind(&mut binding, 7, "A"), Ok(()));
        let mut pending = Some((7, "A"));
        assert_eq!(bind(&mut binding, 7, "B"), Ok(()), "the binding gives way to the window shown");
        assert_eq!(claim(&mut pending, 7, Some(&"B")), Err("target-changed"), "the stale `u` is refused");
        // Once: a `u` of a target that still holds takes it, and the next finds none.
        let mut pending = Some((7, "A"));
        assert_eq!(claim(&mut pending, 7, Some(&"A")), Ok(()));
        assert_eq!(claim(&mut pending, 7, Some(&"A")), Err("target-changed"));
        // Another PID, or nothing shown alone.
        assert_eq!(claim(&mut Some((7, "A")), 8, Some(&"A")), Err("target-changed"));
        assert_eq!(claim(&mut Some((7, "A")), 7, None), Err("target-changed"));
    }

    #[test]
    fn a_targeted_click_goes_only_onto_the_target_with_the_pointer_free() {
        assert_eq!(landing(Some(&1), &1), Ok(()));
        assert_eq!(landing(Some(&2), &1), Err("other-surface-at-point"));
        assert_eq!(landing(None, &1), Err("no-surface-at-point"));
        assert_eq!(free(false, false, false), Ok(()));
        for (grabbed, held, dragging) in [(true, false, false), (false, true, false), (false, false, true)] {
            assert_eq!(free(grabbed, held, dragging), Err("pointer-busy"), "{grabbed} {held} {dragging}");
        }
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
    fn a_u_line_carries_any_text_of_a_person_s_text_message_but_control_characters() {
        let hex = |text: &str| text.bytes().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(composed(&hex("é—中\tline\n")), Ok("é—中\tline\n".to_owned()));
        assert_eq!(composed(&hex(&"中".repeat(display_protocol::MAX_TEXT / 3))).map(|t| t.len()), Ok(1023));
        assert_eq!(composed(&hex(&"a".repeat(display_protocol::MAX_TEXT + 1))), Err("text-too-long"));
        assert_eq!(composed(&hex("a\u{7}")), Err("no-keyboard-resource"));
        assert_eq!(composed("ff"), Err("bad-args"), "not UTF-8");
        assert_eq!(composed("c3"), Err("bad-args"), "a character cut short");
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
