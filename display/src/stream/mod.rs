//! The viewer stream: composed frames out to the admin application, one Unix-socket connection per viewer, and the
//! person's input back in. Only the admin application's uid may connect.

pub mod audio;
mod av1;
mod camera;
mod cursor;
mod encoder;
mod flight;
#[cfg(feature = "h264")]
mod h264;
mod motion;
mod opus;
pub mod record;
mod region;
mod tiles;
mod viewer;
pub use av1::trim_heap;
pub use viewer::Viewers;
mod webm;
mod yuv;

use std::os::unix::net::UnixListener;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use calloop::generic::Generic;
use calloop::{Interest, Mode, PostAction};
use display_protocol::Message;

use crate::compose::Rect;
use crate::state::{Data, State};

/// One composed frame as the encoders see it: compositor-owned pixels and what changed.
pub struct Snapshot {
    pub time_us: u64,
    /// The composed frame itself, shared with the compositor rather than copied.
    pub pixels: Arc<Vec<u32>>,
    pub damage: Vec<Rect>,
    /// Until when the stream should stay in video.
    pub motion_until: Option<Instant>,
}

impl Snapshot {
    pub fn motion_at(&self, now: Instant) -> bool {
        self.motion_until.is_some_and(|until| now <= until)
    }
}

/// What viewer threads tell the compositor.
pub enum Event {
    Input {
        conn: u64,
        message: Message,
    },
    Grant {
        conn: u64,
        epoch: u32,
        holder: bool,
    },
    /// The admin asks the holder's browser to open a page (see `files::open_page`).
    OpenPage {
        conn: u64,
        lease_epoch: u32,
        request: u32,
        url: String,
    },
    Closed {
        conn: u64,
    },
}

#[derive(Default)]
pub struct Hub {
    next_conn: u64,
    motion: motion::Motion,
    pointer: cursor::Pointer,
    /// Every viewer: for frames, the pointer, the browser's sound, and what only the holder gets.
    pub viewers: Arc<viewer::Viewers>,
    cursor_dirty: bool,
    position_dirty: bool,
}

static CLOCK: OnceLock<Instant> = OnceLock::new();

/// Microseconds on the display's monotonic clock, shared by frames and clock calibration.
pub fn clock_us() -> u64 {
    CLOCK.get_or_init(Instant::now).elapsed().as_micros() as u64
}

pub fn listen(handle: &calloop::LoopHandle<'static, Data>, listener: UnixListener, admin_uid: u32) {
    let (sender, receiver) = calloop::channel::channel::<Event>();
    handle
        .insert_source(receiver, |event, _, data: &mut Data| {
            if let calloop::channel::Event::Msg(event) = event {
                handle_event(&mut data.state, event);
                let _ = data.display.flush_clients();
            }
        })
        .expect("stream event source");
    listener.set_nonblocking(true).expect("non-blocking listener");
    handle
        .insert_source(Generic::new(listener, Interest::READ, Mode::Level), move |_, listener, data| {
            while let Ok((stream, _)) = listener.accept() {
                let peer = crate::peer_uid(&stream);
                if peer != Some(admin_uid) {
                    tracing::warn!(?peer, "refused a stream connection from another uid");
                    continue;
                }
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                let state = &mut data.state;
                state.hub.next_conn += 1;
                let conn = state.hub.next_conn;
                let mailbox = Arc::new(viewer::Mailbox::new(state.scene.width, state.scene.height));
                mailbox.offer(Arc::new(snapshot(state, Vec::new())));
                mailbox.offer_pointer(state.hub.pointer.clone());
                viewer::spawn(stream, conn, mailbox, sender.clone(), &state.hub.viewers);
            }
            Ok(PostAction::Continue)
        })
        .expect("stream listener source");
}

fn snapshot(state: &State, damage: Vec<Rect>) -> Snapshot {
    Snapshot {
        time_us: clock_us(),
        pixels: state.scene.framebuffer.clone(),
        damage,
        motion_until: state.hub.motion.hold_ends().filter(|t| *t >= Instant::now()),
    }
}

/// Called after every composition that changed pixels.
pub fn frame_ready(state: &mut State) {
    crate::recording::frame(state);
    let damage = state.scene.frame_damage.clone();
    let area: u64 = damage.iter().map(|r| (r.w * r.h) as u64).sum();
    let output = (state.scene.width * state.scene.height) as u64;
    state.hub.motion.update(Instant::now(), area, output);
    state.hub.viewers.offer(|| Arc::new(snapshot(state, damage)));
}

/// Where the pointer is, and its picture for a recording or the agent's screen copy.
pub fn pointer_now(state: &State) -> ((u16, u16), Option<Arc<record::Picture>>) {
    (pointer_location(state), record::picture(&state.hub.pointer))
}

/// Where the pointer is and how it looks: what of it a copy that overlays it holds.
#[derive(Clone, PartialEq)]
pub struct Shown((u16, u16), cursor::Pointer);

pub fn pointer_shown(state: &State) -> Shown {
    Shown(pointer_location(state), state.hub.pointer.clone())
}

fn pointer_location(state: &State) -> (u16, u16) {
    let at = state.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
    (at.x.max(0.0) as u16, at.y.max(0.0) as u16)
}

/// The cursor changed (hidden, a named shape, a new image, its surface's new pixels). The next frame works out what viewers see, so a client
/// setting its cursor in a loop costs at most one image per frame.
pub fn cursor_changed(state: &mut State) {
    state.hub.cursor_dirty = true;
    state.schedule_frame();
}

/// The pointer moved. Viewers place a custom image themselves, so only then do they need the position; it goes out
/// with the next frame, which also carries the last position after the pointer stops. A named shape follows each
/// viewer's own pointer.
pub fn pointer_moved(state: &mut State) {
    crate::recording::pointer_moved(state, pointer_location(state));
    crate::screencopy::schedule(state);
    if state.hub.pointer.shape == cursor::IMAGE {
        state.hub.position_dirty = true;
        state.schedule_frame();
    }
}

/// Once per rendered frame: offers viewers the pointer if it changed.
pub fn flush_pointer(state: &mut State) {
    let location = pointer_location(state);
    let next = if std::mem::take(&mut state.hub.cursor_dirty) {
        cursor::pointer(&state.cursor, location)
    } else if std::mem::take(&mut state.hub.position_dirty) {
        cursor::Pointer { x: location.0, y: location.1, ..state.hub.pointer.clone() }
    } else {
        return;
    };
    state.hub.position_dirty = false;
    if next == state.hub.pointer {
        return;
    }
    let repictured = !next.same_picture(&state.hub.pointer);
    state.hub.pointer = next;
    // The recording follows the position through `pointer_moved`.
    if repictured {
        crate::recording::pointer_changed(state, || record::picture(&state.hub.pointer));
    }
    state.hub.viewers.offer_pointer(&state.hub.pointer);
}

/// Ends everything of the previous holder: copies, uploads and drops, downloads, the pointer's grabs, the menus it
/// opened, the clipboard.
/// What waited for the text it dropped is told last, so input it then sends lands after the change, not inside it.
fn lease_changed(state: &mut State) {
    crate::copy::cancel(state);
    crate::uploads::lease_changed(state);
    crate::files::lease_changed(state);
    let refuse_typed = crate::input::end_grabs(state);
    // After the keys it holds are released where they were pressed.
    crate::shell::end_popup_grab(state);
    crate::recording::lease_changed(state);
    // Nothing a person pasted stays available once the lease changes hands.
    smithay::wayland::selection::data_device::clear_data_device_selection(&state.dh, &state.seat);
    refuse_typed(state);
}

fn handle_event(state: &mut State, event: Event) {
    match event {
        Event::Grant { conn, epoch, holder } => {
            // The admin application owns the lease; a grant is valid only if it is newer than the current one.
            if epoch <= state.lease.epoch {
                return;
            }
            state.lease.epoch = epoch;
            state.lease.holder = holder.then_some(conn);
            lease_changed(state);
            tracing::info!(epoch, holder = ?state.lease.holder, "control lease changed");
        }
        Event::Closed { conn } => {
            // Its writer ends once nothing holds its queue, sound or not.
            state.hub.viewers.prune();
            if state.lease.holder == Some(conn) {
                state.lease.holder = None;
                state.lease.epoch += 1;
                lease_changed(state);
                tracing::info!(conn, "the control holder disconnected; lease revoked");
            }
        }
        Event::Input { conn, message } => crate::input::person_input(state, conn, message),
        Event::OpenPage { conn, lease_epoch, request, url } => {
            crate::files::open_page(state, conn, lease_epoch, request, url)
        }
    }
}
