//! `zwlr_screencopy_manager_v1` for the agent: a copy of the composed output (or a region of it) written into the
//! agent's own shared-memory buffer. Only clients of the privileged socket can see the global.
//!
//! A copy is made at the next composition, as wlroots makes it, and never inside the request that asks for it: one
//! request is a few bytes, its copy up to the whole output, so copies made as they are asked would let an agent hold
//! the event loop for as many copies as one turn of its requests carries, also while a person holds control. Each
//! composition makes one copy, the oldest asked; a client asks for one more only once its last is made (a further one
//! fails), so what waits is one copy per agent client.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

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

pub const VERSION: u32 = 3;

pub struct ScreencopyGlobal;

/// One requested capture: its output region and whether it was used.
pub struct FrameData {
    region: Rect,
    used: AtomicBool,
}

/// A copy asked for and not made yet.
struct Copy {
    frame: ZwlrScreencopyFrameV1,
    buffer: WlBuffer,
    region: Rect,
}

/// The copies waiting for a composition, oldest first: at most one per live agent client.
#[derive(Default)]
pub struct Copies {
    waiting: VecDeque<Copy>,
}

/// Makes the oldest copy that waits (its frame still there), and has another composition come while more wait.
pub fn make_one(state: &mut State) {
    while let Some(copy) = state.copies.waiting.pop_front() {
        if copy.frame.is_alive() {
            write(state, &copy);
            break;
        }
    }
    if !state.copies.waiting.is_empty() {
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
    if target.write_rows(rows).is_err() {
        frame.failed();
        return;
    }
    if frame.version() >= 2 {
        frame.damage(0, 0, r.w as u32, r.h as u32);
    }
    frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
    let now = state.start.elapsed();
    let secs = now.as_secs();
    frame.ready((secs >> 32) as u32, secs as u32, now.subsec_nanos());
}

impl GlobalDispatch2<ZwlrScreencopyManagerV1, State> for ScreencopyGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ScreencopyGlobal);
    }

    fn can_view(&self, client: &Client) -> bool {
        client_kind(client) == Some(ClientKind::Agent)
    }
}

impl Dispatch2<ZwlrScreencopyManagerV1, State> for ScreencopyGlobal {
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
        let (frame, region) = match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput { frame, .. } => (frame, Some(whole)),
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion { frame, x, y, width, height, .. } => {
                (frame, Rect::new(x, y, width, height).intersect(&whole))
            }
            _ => return,
        };
        let region = region.unwrap_or(Rect::new(0, 0, 0, 0));
        let frame = data_init.init(frame, FrameData { region, used: AtomicBool::new(false) });
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
        let buffer = match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => buffer,
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => buffer,
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
        waiting.push_back(Copy { frame: frame.clone(), buffer, region: r });
        state.schedule_frame();
    }
}
