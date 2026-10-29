//! `zwlr_screencopy_manager_v1` for the agent: a copy of the composed output (or a region of it) written into the
//! agent's own shared-memory buffer. Only clients of the privileged socket can see the global.
//!
//! A copy is made at the next composition, as wlroots makes it, and never inside the request that asks for it: one
//! request is a few bytes, its copy up to the whole output, so copies made as they are asked would let an agent hold
//! the event loop for as many copies as one turn of its requests carries, also while a person holds control. Each
//! composition makes one copy, the oldest that can be made; a client asks for one more only once its last is made (a
//! further one fails), so what waits is one copy per agent client.
//!
//! A copy that overlays the cursor draws the pointer's picture (as a recording draws it) over a copy of the region,
//! never over the framebuffer viewers share.
//!
//! A copy with damage can be made at a composition once its region changed since a frame of its manager last made a
//! copy (a manager's first copy sees the whole output changed), and reports a box around that change. The pointer a
//! copy shows (none without the cursor) changes its whole region when it is not the one that last copy showed.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use smithay::wayland::{Dispatch2, GlobalDispatch2};
use wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};
use wayland_server::protocol::{wl_buffer::WlBuffer, wl_shm};
use wayland_server::{Client, DataInit, DisplayHandle, New, Resource};

use crate::compose::Rect;
use crate::shm;
use crate::state::{ClientKind, State, client_kind};
use crate::stream::Shown;

pub const VERSION: u32 = 3;

pub struct ScreencopyGlobal;

/// What changed since a frame of one manager last made a copy: a box around what the output changed (none when
/// nothing did), and the pointer that copy showed.
#[derive(Default)]
struct Changes {
    output: Option<Rect>,
    pointer: Option<Shown>,
}

type Changed = Arc<Mutex<Changes>>;

/// One bound manager: what changed for its copies with damage.
pub struct Manager {
    changed: Changed,
}

/// One requested capture: its output region, whether it overlays the cursor, its manager's changes, and whether it
/// was used.
pub struct FrameData {
    region: Rect,
    cursor: bool,
    changed: Changed,
    used: AtomicBool,
}

/// A copy asked for and not made yet.
struct Copy {
    frame: ZwlrScreencopyFrameV1,
    buffer: WlBuffer,
    region: Rect,
    cursor: bool,
    changed: Changed,
    with_damage: bool,
}

impl Copy {
    /// The pointer it shows: none unless it overlays the cursor.
    fn pointer(&self, state: &State) -> Option<Shown> {
        self.cursor.then(|| crate::stream::pointer_shown(state))
    }

    /// What changed in its region, in its buffer's coordinates, for a copy showing `pointer`.
    fn damage(&self, pointer: &Option<Shown>) -> Option<Rect> {
        let r = self.region;
        let changes = self.changed.lock().unwrap();
        if changes.pointer != *pointer {
            return Some(Rect::new(0, 0, r.w, r.h));
        }
        let changed = changes.output.and_then(|changed| changed.intersect(&r));
        changed.map(|d| Rect::new(d.x - r.x, d.y - r.y, d.w, d.h))
    }

    /// Whether it can be made at this composition: a copy with damage waits for its region to change.
    fn ready(&self, state: &State) -> bool {
        !self.with_damage || self.damage(&self.pointer(state)).is_some()
    }
}

/// What changed for each manager's copies with damage, kept while the manager or one of its frames is there (a
/// frame outlives its manager), and the copies waiting for a composition, oldest first: at most one per live agent
/// client.
#[derive(Default)]
pub struct Copies {
    changes: Vec<Weak<Mutex<Changes>>>,
    waiting: VecDeque<Copy>,
}

/// Adds what the composition changed to every manager's changes, makes the oldest copy that can be made (its frame
/// still there), and has another composition come while another can.
pub fn make_one(state: &mut State) {
    let copies = &mut state.copies;
    copies.changes.retain(|changes| changes.strong_count() > 0);
    if let Some(changed) = state.scene.frame_damage.iter().copied().reduce(|a, b| a.union(&b)) {
        for changes in copies.changes.iter().filter_map(Weak::upgrade) {
            let mut known = changes.lock().unwrap();
            known.output = Some(known.output.map_or(changed, |known| known.union(&changed)));
        }
    }
    copies.waiting.retain(|copy| copy.frame.is_alive());
    if let Some(i) = state.copies.waiting.iter().position(|copy| copy.ready(state))
        && let Some(copy) = state.copies.waiting.remove(i)
    {
        write(state, &copy);
    }
    schedule(state);
}

/// Has a composition come while a waiting copy can be made: called when the pointer moves too, which changes what a
/// copy showing it holds.
pub fn schedule(state: &mut State) {
    if state.copies.waiting.iter().any(|copy| copy.ready(state)) {
        state.schedule_frame();
    }
}

/// Copies the region of the composed output into the frame's buffer, and tells the client.
fn write(state: &State, copy: &Copy) {
    let (frame, r) = (&copy.frame, copy.region);
    // A buffer the client destroyed meanwhile is no shared-memory buffer any more.
    let Some(target) = shm::buffer_data(&copy.buffer) else {
        frame.failed();
        return;
    };
    let width = state.scene.width as usize;
    let framebuffer = &state.scene.framebuffer;
    let rows = (r.y..r.y + r.h).map(|y| &framebuffer[y as usize * width + r.x as usize..][..r.w as usize]);
    let written = if copy.cursor
        && let ((x, y), Some(picture)) = crate::stream::pointer_now(state)
    {
        let mut pixels: Vec<u32> = rows.flatten().copied().collect();
        picture.draw(&mut pixels, r.w as usize, (i64::from(x) - i64::from(r.x), i64::from(y) - i64::from(r.y)));
        target.write_rows(pixels.chunks(r.w as usize))
    } else {
        target.write_rows(rows)
    };
    if written.is_err() {
        frame.failed();
        return;
    }
    // What the copy holds is what its manager's next copy with damage is told changed from.
    let pointer = copy.pointer(state);
    let damage = copy.damage(&pointer);
    *copy.changed.lock().unwrap() = Changes { output: None, pointer };
    if copy.with_damage
        && let Some(d) = damage
    {
        frame.damage(d.x as u32, d.y as u32, d.w as u32, d.h as u32);
    }
    frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
    let now = state.start.elapsed();
    let secs = now.as_secs();
    frame.ready((secs >> 32) as u32, secs as u32, now.subsec_nanos());
}

impl GlobalDispatch2<ZwlrScreencopyManagerV1, State> for ScreencopyGlobal {
    fn bind(
        &self,
        state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let changed = Arc::new(Mutex::new(Changes { output: Some(state.scene.whole()), pointer: None }));
        state.copies.changes.push(Arc::downgrade(&changed));
        data_init.init(resource, Manager { changed });
    }

    fn can_view(&self, client: &Client) -> bool {
        client_kind(client) == Some(ClientKind::Agent)
    }
}

impl Dispatch2<ZwlrScreencopyManagerV1, State> for Manager {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        let whole = state.scene.whole();
        let (frame, cursor, region) = match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput { frame, overlay_cursor, .. } => {
                (frame, overlay_cursor != 0, Some(whole))
            }
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                x,
                y,
                width,
                height,
                ..
            } => (frame, overlay_cursor != 0, Rect::new(x, y, width, height).intersect(&whole)),
            _ => return,
        };
        let region = region.unwrap_or(Rect::new(0, 0, 0, 0));
        let changed = self.changed.clone();
        let frame = data_init.init(frame, FrameData { region, cursor, changed, used: AtomicBool::new(false) });
        if region.is_empty() {
            frame.failed();
            return;
        }
        frame.buffer(wl_shm::Format::Xrgb8888, region.w as u32, region.h as u32, region.w as u32 * 4);
        if frame.version() >= 3 {
            frame.buffer_done();
        }
    }
}

impl Dispatch2<ZwlrScreencopyFrameV1, State> for FrameData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        let (buffer, with_damage) = match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => (buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => (buffer, true),
            _ => return,
        };
        if self.used.swap(true, Ordering::Relaxed) {
            frame.post_error(zwlr_screencopy_frame_v1::Error::AlreadyUsed, "frame already used");
            return;
        }
        let r = self.region;
        let Some(target) = shm::buffer_data(&buffer) else {
            frame.post_error(zwlr_screencopy_frame_v1::Error::InvalidBuffer, "not a wl_shm buffer");
            return;
        };
        if target.width != r.w as u32 || target.height != r.h as u32 {
            frame.post_error(zwlr_screencopy_frame_v1::Error::InvalidBuffer, "buffer size differs");
            return;
        }
        let waiting = &mut state.copies.waiting;
        waiting.retain(|copy| copy.frame.is_alive());
        if waiting.iter().any(|copy| copy.frame.id().same_client_as(&frame.id())) {
            frame.failed();
            return;
        }
        let copy = Copy {
            frame: frame.clone(),
            buffer,
            region: r,
            cursor: self.cursor,
            changed: self.changed.clone(),
            with_damage,
        };
        waiting.push_back(copy);
        schedule(state);
    }
}
