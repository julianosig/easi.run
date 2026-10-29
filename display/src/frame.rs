//! Frame pacing. A commit is composed as soon as it arrives, at most once per compose gap (which bounds a client that
//! commits in a loop), and the clients that asked are then told to draw their next frame. Composing without waiting
//! for a tick lets Chromium start its next frame right away; Chromium paces itself by the output's advertised refresh.

use std::time::Instant;

use calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{SurfaceAttributes, with_states};
use wayland_server::Resource;

use crate::state::{Data, State};

/// How long without a composition before the compositor's spare frame buffers are released.
pub const IDLE: std::time::Duration = std::time::Duration::from_secs(5);

/// Releases the spare frame buffers of a screen that has not been composed for [`IDLE`].
pub fn release_when_idle(state: &mut State) {
    if state.last_frame.elapsed() >= IDLE && state.scene.release_spares() {
        crate::stream::trim_heap();
    }
}

impl State {
    pub fn schedule_frame(&mut self) {
        if self.scene.frame_scheduled {
            return;
        }
        self.scene.frame_scheduled = true;
        let delay = (self.last_frame + self.compose_gap).saturating_duration_since(Instant::now());
        self.handle
            .insert_source(Timer::from_duration(delay), |_, _, data: &mut Data| {
                render(&mut data.state);
                let _ = data.display.flush_clients();
                TimeoutAction::Drop
            })
            .expect("frame timer");
    }
}

pub fn render(state: &mut State) {
    state.scene.frame_scheduled = false;
    state.last_frame = Instant::now();
    crate::shell::announce_mapping(state);
    let scene = crate::shell::drawn(state);
    let broken = state.scene.compose(&scene);
    for surface in broken {
        state.kill_client_of(&surface, "unreadable shared memory");
    }
    crate::stream::flush_pointer(state);
    if !state.scene.frame_damage.is_empty() {
        crate::stream::frame_ready(state);
    }
    crate::screencopy::make_one(state);
    present_feedback(state, &scene);
    send_callbacks(state, &scene);
}

/// Tells each drawn surface that asked when its content was presented, with the output's refresh interval, so a
/// client such as Chromium times its frames by the display's refresh instead of running them back to back on the
/// frame callbacks that follow each composition.
fn present_feedback(state: &mut State, scene: &[(WlSurface, crate::compose::Rect)]) {
    use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind;
    use smithay::wayland::presentation::{PresentationFeedbackCachedState, Refresh};
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    let time = std::time::Duration::new(now.tv_sec as u64, now.tv_nsec as u32);
    let millihertz = state.output.current_mode().map_or(60_000, |mode| mode.refresh.max(1));
    let refresh = Refresh::fixed(std::time::Duration::from_secs_f64(1000.0 / f64::from(millihertz)));
    state.presentation_seq += 1;
    for (surface, _) in scene {
        let feedbacks = with_states(surface, |states| {
            std::mem::take(&mut states.cached_state.get::<PresentationFeedbackCachedState>().current().callbacks)
        });
        for feedback in feedbacks {
            feedback.presented(&state.output, time, refresh, state.presentation_seq, Kind::Vsync);
        }
    }
}

/// Tells the surfaces that asked (the scene just drawn, a toplevel waiting for its first frame, the cursor) to draw
/// their next frame. A mapped window hidden under the shown one is not told: it would draw frames nobody sees as fast
/// as it is answered. It waits until it is shown, which composes the scene it then belongs to (raising a window or
/// closing the one above it schedules a frame).
fn send_callbacks(state: &mut State, scene: &[(WlSurface, crate::compose::Rect)]) {
    let time = state.now_ms();
    for (surface, _) in scene {
        send_frame(surface, time);
    }
    for top in &state.toplevels {
        let root = top.surface.wl_surface();
        if !state.scene.is_mapped(root) {
            send_frame(root, time);
        }
    }
    if let smithay::input::pointer::CursorImageStatus::Surface(cursor) = &state.cursor {
        send_frame(cursor, time);
    }
}

fn send_frame(surface: &WlSurface, time: u32) {
    if !surface.is_alive() {
        return;
    }
    let callbacks = with_states(surface, |states| {
        std::mem::take(&mut states.cached_state.get::<SurfaceAttributes>().current().frame_callbacks)
    });
    for callback in callbacks {
        callback.done(time);
    }
}
