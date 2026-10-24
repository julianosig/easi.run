//! Input into the browser: the agent's virtual devices and injection socket, and the person's control stream.
//!
//! Every path ends in the same seat. Agent input is refused while a person holds the control lease; a person's input
//! is accepted only from the lease holder's connection at the current lease epoch (see [`Lease`]).

use std::collections::VecDeque;
use std::time::Duration;

use calloop::RegistrationToken;
use calloop::timer::{TimeoutAction, Timer};
use display_protocol::{Message, keys};
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, InputTime, KeyState,
    PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::input::SeatHandler;
use smithay::input::keyboard::{FilterResult, KeyboardHandle, Keycode};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use smithay::wayland::compositor::{SurfaceAttributes, with_states};
use smithay::wayland::virtual_pointer::VirtualPointerBackend;

use crate::state::{Data, State};

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;
/// The last evdev mouse button (`BTN_TASK`); the mouse buttons run from [`BTN_LEFT`] to it.
const BTN_TASK: u32 = 0x117;

/// Whether `code` is an evdev mouse button, the only codes the seat's pointer is pressed with.
pub fn is_mouse_button(code: u32) -> bool {
    (BTN_LEFT..=BTN_TASK).contains(&code)
}

/// Who may inject input. `epoch` changes on every grant and every revocation; input fenced by another epoch is
/// refused.
#[derive(Debug, Default)]
pub struct Lease {
    pub epoch: u32,
    /// The viewer connection that holds control, when a person holds it.
    pub holder: Option<u64>,
}

impl Lease {
    /// Whether `conn` holds control at `epoch`: anything fenced by an older epoch or another connection is stale.
    pub fn holds(&self, conn: u64, epoch: u32) -> bool {
        self.holder == Some(conn) && self.epoch == epoch
    }

    /// The holder's connection and the current epoch, while a person holds control.
    pub fn current(&self) -> Option<(u64, u32)> {
        self.holder.map(|conn| (conn, self.epoch))
    }
}

/// The agent acts only while no person holds control.
pub fn agent_may_act(state: &State) -> bool {
    state.lease.holder.is_none()
}

/// The topmost visible surface that takes input at an output point, with that surface's origin. A surface takes input
/// within its input region (`wl_surface.set_input_region`), the whole surface when it set none: Chromium gives its
/// subsurfaces and tooltips an empty one, so the pointer goes through them to what is below.
pub fn surface_under(state: &State, x: f64, y: f64) -> Option<(WlSurface, Point<f64, Logical>)> {
    crate::shell::scene(state)
        .into_iter()
        .rev()
        .find(|(surface, rect)| rect.contains(x, y) && takes_input(surface, x - rect.x as f64, y - rect.y as f64))
        .map(|(surface, rect)| (surface, (rect.x as f64, rect.y as f64).into()))
}

/// Whether the surface-local point lies in `surface`'s input region.
fn takes_input(surface: &WlSurface, x: f64, y: f64) -> bool {
    with_states(surface, |states| {
        let mut attributes = states.cached_state.get::<SurfaceAttributes>();
        let region = attributes.current().input_region.as_ref();
        region.is_none_or(|region| region.contains((x.floor() as i32, y.floor() as i32)))
    })
}

pub fn clamp_to_output(x: f64, y: f64) -> (f64, f64) {
    let (w, h) = crate::state::output_size();
    (x.clamp(0.0, w as f64 - 1.0), y.clamp(0.0, h as f64 - 1.0))
}

/// Moves the pointer for a person or an agent; while files are dragged over or dropped on the page, only that drag
/// moves it.
pub fn pointer_move(state: &mut State, x: f64, y: f64) {
    if crate::drop::dragging_files(state) {
        return;
    }
    move_pointer(state, x, y);
}

pub fn move_pointer(state: &mut State, x: f64, y: f64) {
    // A NaN would pass the clamp below and become the pointer's location.
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    let Some(pointer) = state.seat.get_pointer() else { return };
    let (x, y) = clamp_to_output(x, y);
    let focus = surface_under(state, x, y);
    let event = MotionEvent { location: (x, y).into(), serial: state.next_serial(), time: InputTime::now() };
    pointer.motion(state, focus, &event);
    pointer.frame(state);
    // A drag's icon follows the pointer in the picture.
    if state.drag_icon.is_some() {
        state.schedule_frame();
    }
    crate::stream::pointer_moved(state);
}

/// Ends what the previous holder's pointer started: a press's click grab (so a late request to drag from it finds no
/// press), a drag the browser started from it, with its icon, and the buttons still held, released where the pointer
/// is, as if the previous holder lifted them at the handoff (the browser keeps a captured button down until it sees
/// the release, so a hidden release would leave it stuck), so the next holder starts with every button up. The keys
/// the person, the injection socket and the agent's virtual keyboards still hold are released the same way: a Control
/// left down would turn the next holder's typing into shortcuts. A locked Caps Lock is unlocked too, so the next
/// holder's shortcuts are not chorded with it. Text still to type, and the input behind it, are dropped first.
///
/// Returns the refusals owed to what waited for the dropped text, for the caller to send once the whole lease change
/// is done: an injection connection told then serves the lines it buffered behind the text, and a press among them
/// sent mid-change would be released by the rest of it.
pub fn end_grabs(state: &mut State) -> impl FnOnce(&mut State) + use<> {
    use smithay::input::keyboard::KeyboardSource;
    let waiting = drop_typing(state);
    if let Some(keyboard) = state.seat.get_keyboard() {
        keyboard.release_source(state, KeyboardSource::MAIN);
        for id in std::mem::take(&mut state.agent_keyboards) {
            keyboard.release_source(state, KeyboardSource::Auxiliary(id));
        }
    }
    unlock_modifiers(state);
    if let Some(pointer) = state.seat.get_pointer() {
        if pointer.is_grabbed() {
            pointer.unset_grab(state, state.next_serial(), InputTime::now());
        }
        state.drag_icon = None;
        for button in std::mem::take(&mut state.buttons) {
            let event = ButtonEvent {
                serial: state.next_serial(),
                time: InputTime::now(),
                button,
                state: ButtonState::Released,
            };
            pointer.button(state, &event);
            pointer.frame(state);
        }
    }
    move |state: &mut State| {
        for then in waiting {
            then(state, false);
        }
    }
}

/// Presses or releases a button for a person or an agent; while files are dragged over or dropped on the page none
/// is, so nothing drops them early.
pub fn pointer_button(state: &mut State, button: u32, pressed: bool) {
    if crate::drop::dragging_files(state) {
        return;
    }
    press(state, button, pressed);
}

/// Presses or releases a button, whatever drag is under way: the files' drop releases its own grab with it.
pub fn press(state: &mut State, button: u32, pressed: bool) {
    let Some(pointer) = state.seat.get_pointer() else { return };
    state.buttons.retain(|&b| b != button);
    if pressed {
        state.buttons.push(button);
    }
    let event = ButtonEvent {
        serial: state.next_serial(),
        time: InputTime::now(),
        button,
        state: if pressed { ButtonState::Pressed } else { ButtonState::Released },
    };
    pointer.button(state, &event);
    pointer.frame(state);
}

/// Scrolls by `dx`, `dy` pixels. The high-resolution wheel amount carries the pixels themselves: Chromium scrolls one
/// pixel per v120 unit and prefers that amount over the axis value, so a fixed notch of 120 per event turned every small
/// trackpad delta into a 120-pixel jump.
pub fn pointer_scroll(state: &mut State, dx: f64, dy: f64) {
    axis(state, dx, dy, |amount| amount.round() as i32);
}

/// Scrolls one wheel notch per direction, whatever the axis value: the automation agent's virtual-pointer convention.
pub fn pointer_notch(state: &mut State, dx: f64, dy: f64) {
    axis(state, dx, dy, |amount| if amount < 0.0 { -120 } else { 120 });
}

fn axis(state: &mut State, dx: f64, dy: f64, v120: impl Fn(f64) -> i32) {
    let Some(pointer) = state.seat.get_pointer() else { return };
    let mut frame = AxisFrame::new(InputTime::now()).source(AxisSource::Wheel);
    for (axis, amount) in [(Axis::Horizontal, dx), (Axis::Vertical, dy)] {
        if amount != 0.0 {
            frame = frame.value(axis, amount).v120(axis, v120(amount));
        }
    }
    pointer.axis(state, frame);
    pointer.frame(state);
}

/// Presses or releases a Linux evdev key on the seat keyboard.
pub fn key(state: &mut State, evdev_code: u32, pressed: bool) {
    let Some(keyboard) = state.seat.get_keyboard() else { return };
    let serial = state.next_serial();
    let time = InputTime::now();
    let key_state = if pressed { KeyState::Pressed } else { KeyState::Released };
    keyboard
        .input::<(), _>(state, Keycode::new(evdev_code + 8), key_state, serial, time, |_, _, _| FilterResult::Forward);
}

pub fn agent_pointer_event(state: &mut State, event: InputEvent<VirtualPointerBackend>) {
    if !agent_may_act(state) {
        return;
    }
    handle_pointer_event(state, event);
}

fn handle_pointer_event<B: InputBackend>(state: &mut State, event: InputEvent<B>) {
    match event {
        InputEvent::PointerMotion { event } => {
            let Some(pointer) = state.seat.get_pointer() else { return };
            let at = pointer.current_location() + event.delta();
            pointer_move(state, at.x, at.y);
        }
        InputEvent::PointerMotionAbsolute { event } => {
            let (w, h) = crate::state::output_size();
            let at = event.position_transformed((w as i32, h as i32).into());
            pointer_move(state, at.x, at.y);
        }
        // Only mouse buttons: another code is no button a page knows, and each one held would grow the held buttons.
        InputEvent::PointerButton { event } if is_mouse_button(event.button_code()) => {
            pointer_button(state, event.button_code(), event.state() == ButtonState::Pressed);
        }
        InputEvent::PointerAxis { event } => {
            let Some(pointer) = state.seat.get_pointer() else { return };
            let mut frame = AxisFrame::new(event.time()).source(event.source());
            for axis in [Axis::Horizontal, Axis::Vertical] {
                if let Some(amount) = event.amount(axis) {
                    frame = frame.value(axis, amount);
                }
                if let Some(v120) = event.amount_v120(axis) {
                    frame = frame.v120(axis, v120 as i32);
                }
            }
            pointer.axis(state, frame);
            pointer.frame(state);
        }
        _ => {}
    }
}

/// DOM `MouseEvent.button` numbering to evdev buttons.
fn dom_button(button: u8) -> u32 {
    match button {
        1 => BTN_MIDDLE,
        2 => BTN_RIGHT,
        _ => BTN_LEFT,
    }
}

/// Highest evdev key code a person may press.
const MAX_KEY_CODE: u16 = keys::MAX;

/// A person's input from a viewer connection, applied only for the lease holder at the current epoch. Input that acts
/// on the seat waits behind text still being typed, so it lands where the text ends (a click after a long text does
/// not move the caret before the text is in).
pub fn person_input(state: &mut State, conn: u64, message: Message) {
    let seat = matches!(
        message,
        Message::Pointer { .. }
            | Message::Wheel { .. }
            | Message::Key { .. }
            | Message::Clipboard { .. }
            | Message::FileDrop { .. }
            | Message::FileDrag { .. }
            | Message::FileDragEnd { .. }
    );
    if !(seat && typing(state)) {
        return apply_person_input(state, conn, message);
    }
    if state.typing.behind.len() >= MAX_BEHIND {
        tracing::warn!(conn, "dropped input behind text still being typed");
        return;
    }
    state.typing.behind.push_back((conn, message));
}

fn apply_person_input(state: &mut State, conn: u64, message: Message) {
    use display_protocol::{KeyKind, PointerKind};
    let Some(epoch) = message.lease_epoch() else { return };
    if !state.lease.holds(conn, epoch) {
        tracing::warn!(conn, epoch, current = state.lease.epoch, "refused input from a connection without the lease");
        return;
    }
    match message {
        Message::Pointer { kind, button, x, y, .. } => {
            pointer_move(state, f64::from(x), f64::from(y));
            match kind {
                PointerKind::Move => {}
                PointerKind::Down => pointer_button(state, dom_button(button), true),
                PointerKind::Up => pointer_button(state, dom_button(button), false),
            }
        }
        Message::Wheel { x, y, dx, dy, .. } => {
            pointer_move(state, f64::from(x), f64::from(y));
            pointer_scroll(state, f64::from(dx), f64::from(dy));
        }
        Message::Key { kind: KeyKind::Down, code, .. } if code > 0 && code <= MAX_KEY_CODE => {
            key(state, code.into(), true)
        }
        Message::Key { kind: KeyKind::Up, code, .. } if code > 0 && code <= MAX_KEY_CODE => {
            key(state, code.into(), false)
        }
        Message::Key { kind: KeyKind::Text, text, .. } => type_text(state, &text),
        Message::Clipboard { text, .. } => paste(state, text),
        Message::FileRead { file, offset, .. } => crate::files::read(state, conn, file, offset),
        Message::FileCancel { file, .. } => crate::files::cancel(state, conn, file),
        Message::UploadStart { upload, size, name, .. } => {
            crate::uploads::start(state, conn, epoch, upload, size, &name)
        }
        Message::UploadData { upload, offset, data, .. } => {
            crate::uploads::data(state, conn, epoch, upload, offset, &data)
        }
        Message::FileChoose { request, uploads, .. } => crate::uploads::choose(state, conn, epoch, request, uploads),
        Message::FileDrop { x, y, uploads, .. } => crate::drop::drop_files(state, conn, epoch, x, y, uploads),
        Message::FileDrag { x, y, types, .. } => crate::drop::drag_files(state, conn, epoch, x, y, types),
        Message::FileDragEnd { .. } => crate::drop::drag_ended(state),
        Message::RecordStart { recording, layout, camera, microphone, page_sound, .. } => {
            let request = crate::recording::Request { id: recording, layout, camera, microphone, page_sound };
            crate::recording::start(state, conn, request)
        }
        Message::RecordCamera { recording, timestamp_us, jpeg, .. } => {
            crate::recording::camera(state, conn, recording, timestamp_us, jpeg)
        }
        Message::RecordMicrophone { recording, timestamp_us, payload, .. } => {
            crate::recording::microphone(state, conn, recording, timestamp_us, payload)
        }
        Message::RecordStop { recording, .. } => crate::recording::stop(state, conn, recording),
        Message::RecordLayout { recording, layout, .. } => crate::recording::layout(state, conn, recording, layout),
        _ => {}
    }
}

/// Replaces the browser's clipboard with the person's text, as a selection the compositor owns, and pastes it once.
fn paste(state: &mut State, text: String) {
    const KEY_LEFTCTRL: u32 = keys::LEFTCTRL as u32;
    const KEY_V: u32 = keys::V as u32;
    let mime_types = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"].map(String::from).to_vec();
    smithay::wayland::selection::data_device::set_data_device_selection(
        &state.dh,
        &state.seat,
        mime_types,
        std::sync::Arc::from(text),
    );
    // The selection event reaches the focused client before these keys, on the same connection. A Control the person
    // already holds (their own paste shortcut) stays held: releasing it here would turn their next chord into typing.
    let held = state.seat.get_keyboard().is_some_and(|keyboard| keyboard.modifier_state().ctrl);
    if !held {
        key(state, KEY_LEFTCTRL, true);
    }
    key(state, KEY_V, true);
    key(state, KEY_V, false);
    if !held {
        key(state, KEY_LEFTCTRL, false);
    }
}

/// Strokes typed in one event-loop turn. Every key event goes into the browser's socket pair before the relay can
/// forward any of it, so a whole long text at once (some 2,000 letters, or 650 with Shift) filled the pair and the
/// library's bounded buffer, and the library disconnected the browser. A slice this size stays far below the pair's
/// room with Shift around every key.
const STROKES_PER_TURN: usize = 256;
/// What a keymap switch counts for against [`STROKES_PER_TURN`]. The keyboard compiles each keymap it is given (some
/// 2 ms), and so does the browser for each one it receives; smithay compiles from text only, so a compiled keymap
/// cannot be kept for the next time a character comes back. A text of ever different characters (Chinese, or accents
/// in turn) therefore switches at most 8 times per turn instead of holding the loop half a second per slice.
const SWITCH_COST: usize = 32;
/// The pause between two slices of a long text, so the relay forwards one before the next is typed.
const TYPING_PAUSE: Duration = Duration::from_millis(2);
/// A person's seat input kept behind text still being typed: far more than a person sends in the milliseconds a text
/// takes, so only a flood is dropped, and the pastes it can hold stay within 16 MiB.
const MAX_BEHIND: usize = 256;

/// One character to type: an ASCII key (with Shift or not) or a character the spare key types.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stroke {
    /// The key and whether it needs Shift.
    Ascii((u32, bool)),
    Spare(char),
}

/// What waits for typed text, told whether the text went in.
type WhenTyped = Box<dyn FnOnce(&mut State, bool)>;

/// Text being typed a slice per turn (see [`STROKES_PER_TURN`]), with what waits for it.
#[derive(Default)]
pub struct Typing {
    strokes: VecDeque<Stroke>,
    /// The character the spare key types now, when the keyboard left the base keymap.
    spare: Option<char>,
    /// A person's seat input that came while text was being typed, applied in order once it is in.
    behind: VecDeque<(u64, Message)>,
    /// Called once the text is typed (`true`) or dropped by a lease change (`false`): the injection socket's answers.
    waiting: Vec<WhenTyped>,
    /// The next slice's timer, while text, input, or an answer waits for it.
    timer: Option<RegistrationToken>,
}

/// Whether text is still to be typed over the next turns.
pub fn typing(state: &State) -> bool {
    !state.typing.strokes.is_empty()
}

/// Calls `then` once the text being typed is in, or dropped (`false`) because control changed hands.
pub fn when_typed(state: &mut State, then: impl FnOnce(&mut State, bool) + 'static) {
    state.typing.waiting.push(Box::new(then));
}

/// Types composed text: each character the seat's keymap has a key for is pressed with that key; any other is typed
/// with the spare key, and the base keymap is restored once the text is in.
fn type_text(state: &mut State, text: &str) {
    let strokes = text
        .chars()
        .map(|c| u8::try_from(c).ok().and_then(|b| state.chars.get(b)).map_or(Stroke::Spare(c), Stroke::Ascii));
    type_strokes(state, strokes.collect());
}

/// Types ASCII keys from the [`crate::inject::CharTable`], the injection socket's text.
pub fn type_ascii_text(state: &mut State, keys: Vec<(u32, bool)>) {
    type_strokes(state, keys.into_iter().map(Stroke::Ascii).collect());
}

/// Types a slice now and the rest over the next turns, after any text already waiting.
fn type_strokes(state: &mut State, strokes: Vec<Stroke>) {
    state.typing.strokes.extend(strokes);
    if state.typing.timer.is_some() {
        return;
    }
    type_slice(state);
    if state.typing.strokes.is_empty() {
        restore_keymap(state);
        return;
    }
    let timer = state
        .handle
        .insert_source(Timer::from_duration(TYPING_PAUSE), |_, _, data: &mut Data| next_slice(&mut data.state));
    match timer {
        Ok(timer) => state.typing.timer = Some(timer),
        Err(e) => {
            tracing::error!(%e, "the rest of a text could not be scheduled; it was not typed");
            state.typing.strokes.clear();
            restore_keymap(state);
        }
    }
}

/// Types the next slice; once the text is in, answers what waits for it and applies the input that came behind it,
/// until that input types text of its own.
fn next_slice(state: &mut State) -> TimeoutAction {
    type_slice(state);
    loop {
        if !state.typing.strokes.is_empty() {
            return TimeoutAction::ToDuration(TYPING_PAUSE);
        }
        restore_keymap(state);
        // What waited for the text may type more of its own, which this timer then types.
        let waiting = std::mem::take(&mut state.typing.waiting);
        if !waiting.is_empty() {
            for then in waiting {
                then(state, true);
            }
            continue;
        }
        let Some((conn, message)) = state.typing.behind.pop_front() else {
            state.typing.timer = None;
            return TimeoutAction::Drop;
        };
        apply_person_input(state, conn, message);
    }
}

fn type_slice(state: &mut State) {
    unlock_modifiers(state);
    let mut budget = STROKES_PER_TURN;
    while budget > 0 {
        let cost = match state.typing.strokes.pop_front() {
            Some(Stroke::Ascii(key)) => {
                type_ascii(state, key);
                1
            }
            Some(Stroke::Spare(c)) if type_unicode(state, c) => SWITCH_COST,
            Some(Stroke::Spare(_)) => 1,
            None => break,
        };
        budget = budget.saturating_sub(cost);
    }
}

/// Drops the text still to type and the input behind it: the previous holder's, at a lease change. Returns what
/// waited for the text, still to be told it was not typed.
fn drop_typing(state: &mut State) -> Vec<WhenTyped> {
    let typing = std::mem::take(&mut state.typing);
    if let Some(timer) = typing.timer {
        state.handle.remove(timer);
    }
    state.typing.spare = typing.spare;
    restore_keymap(state);
    typing.waiting
}

/// Gives the spare key back its base keymap once no text needs it.
fn restore_keymap(state: &mut State) {
    if state.typing.spare.take().is_some()
        && let Some(keyboard) = state.seat.get_keyboard()
        && keyboard.set_keymap_from_string(state, state.chars.keymap.clone()).is_err()
    {
        tracing::error!("the base keymap could not be restored");
    }
}

/// Unlocks Caps Lock and every other locked modifier. Text arrives already cased (a person's keys type through the
/// viewer's own layout and Caps Lock, the agent names its characters), and a locked Caps Lock would invert the case of
/// every letter typed with or without Shift, and of a character the spare key types. Nothing else needs it locked:
/// a person's printable keys arrive as text, and a Caps Lock a Mac viewer pressed may never report its release. A
/// Caps Lock still held down is released first: its Lock would stay depressed, and inverted, after any unlocking.
pub fn unlock_modifiers(state: &mut State) {
    let Some(keyboard) = state.seat.get_keyboard() else { return };
    let caps = Keycode::new(u32::from(keys::CAPSLOCK) + 8);
    if keyboard.pressed_keys().contains(&caps) {
        // Whichever keyboard holds it: the person's and the injection socket's, or one of the agent's virtual ones (a
        // release from another source would be absorbed and leave Lock depressed).
        use smithay::input::keyboard::KeyboardSource;
        let sources = std::iter::once(KeyboardSource::MAIN)
            .chain(state.agent_keyboards.iter().map(|id| KeyboardSource::Auxiliary(*id)))
            .collect::<Vec<_>>();
        for source in sources {
            let serial = state.next_serial();
            keyboard.input_from_source::<(), _>(
                source,
                state,
                caps,
                KeyState::Released,
                serial,
                InputTime::now(),
                |_, _, _| FilterResult::Forward,
            );
        }
    }
    let mods = keyboard.modifier_state().serialized;
    if mods.locked != 0 {
        keyboard.with_xkb_state(state, |mut context| {
            context.set_modifier_mask(mods.depressed, mods.latched, 0, mods.layout_effective)
        });
    }
}

/// Types one ASCII character with its key from the [`crate::inject::CharTable`], with Shift when it needs it.
fn type_ascii(state: &mut State, (code, shift): (u32, bool)) {
    let Some(keyboard) = state.seat.get_keyboard() else { return };
    tap_alone(&keyboard, state, code, shift);
}

/// xkb's modifier mask for Shift: every keymap numbers the eight core modifiers first, Shift at index 0.
const SHIFT_MASK: u32 = 1;

/// Presses and releases `code` with Shift exactly as its character needs and no other modifier, then gives back the
/// modifiers held before. Text arrives already cased and composed while the person may still hold the Shift (or AltGr)
/// that produced it, sent as a key of its own: under that held Shift the seat would type an AZERTY viewer's `75001` as
/// `&%))!` and a German one's `/` as `?`, and a held AltGr (Alt here) would turn an `@` into a shortcut. Setting the
/// mask instead of pressing a Shift key also leaves the person's own Shift held (pressing and releasing the Shift they
/// hold would release it), so a Shift+arrow or Shift+click after the text still extends the selection. The browser is
/// sent the stroke's modifiers before its key and the held ones after it, and resolves the key by them.
fn tap_alone<D: SeatHandler + 'static>(keyboard: &KeyboardHandle<D>, data: &mut D, code: u32, shift: bool) {
    let held = keyboard.modifier_state().serialized;
    let group = held.layout_effective;
    let stroke = if shift { SHIFT_MASK } else { 0 };
    keyboard.with_xkb_state(data, |mut xkb| xkb.set_modifier_mask(stroke, 0, 0, group));
    for key_state in [KeyState::Pressed, KeyState::Released] {
        let serial = SERIAL_COUNTER.next_serial();
        keyboard.input::<(), _>(data, Keycode::new(code + 8), key_state, serial, InputTime::now(), |_, _, _| {
            FilterResult::Forward
        });
    }
    keyboard.with_xkb_state(data, |mut xkb| xkb.set_modifier_mask(held.depressed, held.latched, held.locked, group));
}

/// The evdev code that types a character the US layout has no key for: the ISO keyboard's extra key (`KEY_102ND`, xkb
/// `<LSGT>`), an ordinary printable key, so the browser treats it as typing rather than as a special key.
const SPARE_KEY: u32 = keys::KEY_102ND as u32;

/// Types one character without a key of its own (`ç`, `ã`, `€`, ...) with the spare key, switching first to a keymap
/// whose spare key produces it unless the keyboard already has that one. Every other key types as in the base keymap,
/// so the switch back waits for the end of the text. Returns whether a keymap was compiled for it.
fn type_unicode(state: &mut State, c: char) -> bool {
    let switch = state.typing.spare != Some(c);
    if switch {
        let Some(keyboard) = state.seat.get_keyboard() else { return false };
        let Some(keymap) = keymap_with(&state.chars.keymap, c) else { return false };
        if keyboard.set_keymap_from_string(state, keymap).is_err() {
            tracing::warn!("a character could not be given a key; it was not typed");
            return true;
        }
        state.typing.spare = Some(c);
    }
    // After the switch, which derives the new keymap's modifiers from the keys held: a held Control would make the
    // character a shortcut.
    if let Some(keyboard) = state.seat.get_keyboard() {
        tap_alone(&keyboard, state, SPARE_KEY, false);
    }
    switch
}

/// The base keymap with the spare key producing `c`.
pub fn keymap_with(base: &str, c: char) -> Option<String> {
    if c.is_control() {
        return None;
    }
    let symbols = base.find("xkb_symbols")?;
    let end = symbols + base[symbols..].find("\n};")?;
    let line = format!("\n\tkey <LSGT> {{ [ U{:04X} ] }};", c as u32);
    Some(format!("{}{line}{}", &base[..end], &base[end..]))
}

#[cfg(test)]
mod tests {
    use smithay::backend::input::{InputTime, KeyState};
    use smithay::input::keyboard::{FilterResult, KeyboardTarget, Keycode, KeysymHandle, ModifiersState, xkb};
    use smithay::input::pointer::{self, AxisFrame, ButtonEvent, MotionEvent, PointerTarget, RelativeMotionEvent};
    use smithay::input::touch::{self, FrameMarker, TouchTarget};
    use smithay::input::{Seat, SeatHandler, SeatState};
    use smithay::utils::{IsAlive, SERIAL_COUNTER, Serial};

    /// What the focused client was sent: a key with the character the seat resolved it to, or modifiers (Shift, Control).
    #[derive(Debug, PartialEq)]
    enum Sent {
        Key(char, KeyState),
        Modifiers(bool, bool),
    }

    struct Seated {
        seat_state: SeatState<Seated>,
        sent: Vec<Sent>,
    }

    /// The focused client, recording what it is sent.
    #[derive(Debug, Clone, PartialEq)]
    struct Client;

    impl IsAlive for Client {
        fn alive(&self) -> bool {
            true
        }
    }

    impl KeyboardTarget<Seated> for Client {
        fn enter(&self, _: &Seat<Seated>, _: &mut Seated, _: Vec<KeysymHandle<'_>>, _: Serial) {}
        fn leave(&self, _: &Seat<Seated>, _: &mut Seated, _: Serial) {}
        fn key(
            &self,
            _: &Seat<Seated>,
            data: &mut Seated,
            key: KeysymHandle<'_>,
            state: KeyState,
            _: Serial,
            _: InputTime,
        ) {
            let c = key.modified_sym().key_char().unwrap_or('?');
            data.sent.push(Sent::Key(c, state));
        }
        fn modifiers(&self, _: &Seat<Seated>, data: &mut Seated, modifiers: ModifiersState, _: Serial) {
            data.sent.push(Sent::Modifiers(modifiers.shift, modifiers.ctrl));
        }
    }

    impl PointerTarget<Seated> for Client {
        fn enter(&self, _: &Seat<Seated>, _: &mut Seated, _: &MotionEvent) {}
        fn motion(&self, _: &Seat<Seated>, _: &mut Seated, _: &MotionEvent) {}
        fn relative_motion(&self, _: &Seat<Seated>, _: &mut Seated, _: &RelativeMotionEvent) {}
        fn button(&self, _: &Seat<Seated>, _: &mut Seated, _: &ButtonEvent) {}
        fn axis(&self, _: &Seat<Seated>, _: &mut Seated, _: AxisFrame) {}
        fn frame(&self, _: &Seat<Seated>, _: &mut Seated) {}
        fn gesture_swipe_begin(&self, _: &Seat<Seated>, _: &mut Seated, _: &pointer::GestureSwipeBeginEvent) {}
        fn gesture_swipe_update(&self, _: &Seat<Seated>, _: &mut Seated, _: &pointer::GestureSwipeUpdateEvent) {}
        fn gesture_swipe_end(&self, _: &Seat<Seated>, _: &mut Seated, _: &pointer::GestureSwipeEndEvent) {}
        fn gesture_pinch_begin(&self, _: &Seat<Seated>, _: &mut Seated, _: &pointer::GesturePinchBeginEvent) {}
        fn gesture_pinch_update(&self, _: &Seat<Seated>, _: &mut Seated, _: &pointer::GesturePinchUpdateEvent) {}
        fn gesture_pinch_end(&self, _: &Seat<Seated>, _: &mut Seated, _: &pointer::GesturePinchEndEvent) {}
        fn gesture_hold_begin(&self, _: &Seat<Seated>, _: &mut Seated, _: &pointer::GestureHoldBeginEvent) {}
        fn gesture_hold_end(&self, _: &Seat<Seated>, _: &mut Seated, _: &pointer::GestureHoldEndEvent) {}
        fn leave(&self, _: &Seat<Seated>, _: &mut Seated, _: Serial, _: InputTime) {}
    }

    impl TouchTarget<Seated> for Client {
        fn down(&self, _: &Seat<Seated>, _: &mut Seated, _: &touch::DownEvent) {}
        fn up(&self, _: &Seat<Seated>, _: &mut Seated, _: &touch::UpEvent) {}
        fn motion(&self, _: &Seat<Seated>, _: &mut Seated, _: &touch::MotionEvent) {}
        fn frame(&self, _: &Seat<Seated>, _: &mut Seated, _: FrameMarker) {}
        fn cancel(&self, _: &Seat<Seated>, _: &mut Seated, _: FrameMarker) {}
        fn shape(&self, _: &Seat<Seated>, _: &mut Seated, _: &touch::ShapeEvent) {}
        fn orientation(&self, _: &Seat<Seated>, _: &mut Seated, _: &touch::OrientationEvent) {}
        fn last_frame(&self, _: &Seat<Seated>, _: &mut Seated) -> Option<FrameMarker> {
            None
        }
    }

    impl SeatHandler for Seated {
        type KeyboardFocus = Client;
        type PointerFocus = Client;
        type TouchFocus = Client;
        fn seat_state(&mut self) -> &mut SeatState<Seated> {
            &mut self.seat_state
        }
    }

    fn hold(
        keyboard: &smithay::input::keyboard::KeyboardHandle<Seated>,
        data: &mut Seated,
        code: u16,
        state: KeyState,
    ) {
        let serial = SERIAL_COUNTER.next_serial();
        let code = Keycode::new(u32::from(code) + 8);
        keyboard.input::<(), _>(data, code, state, serial, InputTime::now(), |_, _, _| FilterResult::Forward);
    }

    #[test]
    fn text_types_with_only_the_shift_it_needs_and_leaves_held_modifiers_held() {
        use display_protocol::keys;
        let mut data = Seated { seat_state: SeatState::new(), sent: Vec::new() };
        let mut seat = data.seat_state.new_seat("test");
        let keyboard = seat.add_keyboard(Default::default(), 600, 0).unwrap();
        keyboard.set_focus(&mut data, Some(Client), SERIAL_COUNTER.next_serial());
        let table = crate::inject::CharTable::new();
        let typed = |data: &mut Seated, c: u8| {
            data.sent.clear();
            let (code, shift) = table.get(c).unwrap();
            super::tap_alone(&keyboard, data, code, shift);
            std::mem::take(&mut data.sent)
        };
        let (pressed, released) = (KeyState::Pressed, KeyState::Released);

        // Nothing held: a lower-case letter is a bare key, an upper-case one is Shift'ed and Shift goes with it.
        assert_eq!(typed(&mut data, b'a'), [Sent::Key('a', pressed), Sent::Key('a', released)]);
        assert_eq!(
            typed(&mut data, b'A'),
            [
                Sent::Modifiers(true, false),
                Sent::Key('A', pressed),
                Sent::Key('A', released),
                Sent::Modifiers(false, false)
            ]
        );

        // The person holds the Shift that made their AZERTY `7`: the digit is typed without it, and it is held after.
        hold(&keyboard, &mut data, keys::LEFTSHIFT, pressed);
        assert_eq!(
            typed(&mut data, b'7'),
            [
                Sent::Modifiers(false, false),
                Sent::Key('7', pressed),
                Sent::Key('7', released),
                Sent::Modifiers(true, false)
            ]
        );
        assert_eq!(typed(&mut data, b'?'), [Sent::Key('?', pressed), Sent::Key('?', released)]);
        assert!(keyboard.modifier_state().shift, "the person's Shift is still held");
        hold(&keyboard, &mut data, keys::LEFTSHIFT, released);
        assert!(!keyboard.modifier_state().shift, "releasing it leaves no Shift behind");

        // A held Control never reaches the browser with a character.
        hold(&keyboard, &mut data, keys::LEFTCTRL, pressed);
        assert_eq!(
            typed(&mut data, b'@'),
            [
                Sent::Modifiers(true, false),
                Sent::Key('@', pressed),
                Sent::Key('@', released),
                Sent::Modifiers(false, true)
            ]
        );
        hold(&keyboard, &mut data, keys::LEFTCTRL, released);
        assert_eq!(keyboard.modifier_state().serialized.depressed, 0);
    }

    #[test]
    fn a_spare_key_types_characters_the_layout_lacks() {
        let base = crate::inject::CharTable::new().keymap;
        assert!(base.contains("<LSGT>"), "the evdev keycodes name the ISO extra key");
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        for c in ['ç', 'ã', 'é', '€', '中'] {
            let text = super::keymap_with(&base, c).unwrap();
            let keymap =
                xkb::Keymap::new_from_string(&context, text, xkb::KEYMAP_FORMAT_TEXT_V1, xkb::KEYMAP_COMPILE_NO_FLAGS)
                    .expect("the extended keymap compiles");
            let syms = keymap.key_get_syms_by_level((super::SPARE_KEY + 8).into(), 0, 0);
            assert_eq!(syms.len(), 1);
            assert_eq!(xkb::keysym_to_utf32(syms[0]), c as u32);
        }
        assert!(super::keymap_with(&base, '\u{7}').is_none());
        // Text keeps a switched keymap until its end, so no character the base keymap types may need the spare key.
        let table = crate::inject::CharTable::new();
        assert!((0..128u8).all(|c| table.get(c).is_none_or(|(code, _)| code != super::SPARE_KEY)));
    }
}
