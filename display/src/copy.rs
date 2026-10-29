//! Text the browser copies, to the person holding control.
//!
//! Only the focused client can set the clipboard (Smithay refuses others), and the browser is the only client with
//! surfaces, so a selection always comes from the browser. When a person holds control, the display reads the text
//! through a pipe (non-blocking, at most [`MAX_COPIED`] bytes, within [`DEADLINE`]) and sends it to that connection
//! alone, at the lease epoch of the copy: a lease change before the text arrives drops it, and a copy made while the
//! agent holds control is never collected. The client puts it on the holder's clipboard at once where their browser
//! lets a page write it, and otherwise offers it on a click.

use std::time::{Duration, Instant};

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{Interest, Mode, PostAction, RegistrationToken};
use display_protocol::{MAX_COPIED, Message};
use rustix::pipe::PipeFlags;
use smithay::wayland::selection::SelectionSource;
use smithay::wayland::selection::data_device::request_data_device_client_selection;

use crate::state::{Data, State};

/// Time the browser has to write the copied text.
const DEADLINE: Duration = Duration::from_secs(2);
/// Text types, in order of preference: what a copy takes, and a paste offers.
pub(crate) const TEXT_TYPES: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];

/// How often a copy the holder's queue had no room for is tried again, and for how long.
const RETRY: Duration = Duration::from_millis(100);
const RETRY_FOR: Duration = Duration::from_secs(10);

/// The copy in progress: the request waiting for the loop to be idle, the pipe being read with its deadline, or the
/// text waiting for room in the holder's queue. At most one of each, always the latest copy.
#[derive(Default)]
pub struct Job {
    idle: Option<calloop::Idle<'static>>,
    sources: Option<(RegistrationToken, RegistrationToken)>,
    retry: Option<RegistrationToken>,
}

/// Drops the copy in progress; a lease change calls this, so no text crosses to the next holder.
pub fn cancel(state: &mut State) {
    if let Some(idle) = state.copy.idle.take() {
        idle.cancel();
    }
    stop_reading(state);
    if let Some(retry) = state.copy.retry.take() {
        state.handle.remove(retry);
    }
}

fn stop_reading(state: &mut State) {
    if let Some((pipe, timer)) = state.copy.sources.take() {
        state.handle.remove(pipe);
        state.handle.remove(timer);
    }
}

/// Ends the job from inside its pipe's callback, which removes the pipe itself by returning `Remove`.
fn end_from_pipe(state: &mut State) {
    if let Some((_, timer)) = state.copy.sources.take() {
        state.handle.remove(timer);
    }
}

/// The browser set the clipboard to `source`. Smithay records the new selection after this returns, so the text is
/// requested once the loop is idle.
pub fn selection_changed(state: &mut State, source: Option<SelectionSource>) {
    cancel(state);
    let (Some(source), Some(conn)) = (source, state.lease.holder) else { return };
    let offered = source.mime_types();
    let Some(mime) = TEXT_TYPES.iter().find(|t| offered.iter().any(|o| o == *t)) else { return };
    let epoch = state.lease.epoch;
    // A burst of copies requests the selection once, the latest.
    state.copy.idle = Some(state.handle.insert_idle(move |data| {
        data.state.copy.idle = None;
        read(&mut data.state, conn, epoch, mime);
    }));
}

/// Reads the clipboard's text as `mime` for `conn`, if it still holds control at `epoch`.
fn read(state: &mut State, conn: u64, epoch: u32, mime: &'static str) {
    cancel(state);
    if !state.lease.holds(conn, epoch) {
        return;
    }
    let Ok((read, write)) = rustix::pipe::pipe_with(PipeFlags::CLOEXEC | PipeFlags::NONBLOCK) else { return };
    if let Err(e) = request_data_device_client_selection(&state.seat, mime.to_owned(), write) {
        tracing::info!(%e, "the browser's copied text could not be requested");
        return;
    }
    let mut text: Vec<u8> = Vec::new();
    let pipe =
        state.handle.insert_source(Generic::new(read, Interest::READ, Mode::Level), move |_, fd, data: &mut Data| {
            let mut chunk = [0u8; 64 * 1024];
            loop {
                match rustix::io::read(&*fd, &mut chunk) {
                    Ok(0) => {
                        deliver(&mut data.state, conn, epoch, std::mem::take(&mut text));
                        end_from_pipe(&mut data.state);
                        return Ok(PostAction::Remove);
                    }
                    Ok(n) if text.len() + n > MAX_COPIED => {
                        tracing::info!("the browser copied more text than a person may receive; not sent");
                        end_from_pipe(&mut data.state);
                        return Ok(PostAction::Remove);
                    }
                    Ok(n) => text.extend_from_slice(&chunk[..n]),
                    Err(rustix::io::Errno::AGAIN) => return Ok(PostAction::Continue),
                    Err(rustix::io::Errno::INTR) => {}
                    Err(_) => {
                        end_from_pipe(&mut data.state);
                        return Ok(PostAction::Remove);
                    }
                }
            }
        });
    let Ok(pipe) = pipe else { return };
    let started = Instant::now();
    let timer = state.handle.insert_source(Timer::from_duration(DEADLINE), move |_, _, data: &mut Data| {
        tracing::info!(waited = ?started.elapsed(), "the browser did not finish writing the copied text");
        // The timer drops itself; the pipe goes with it.
        if let Some((pipe, _)) = data.state.copy.sources.take() {
            data.state.handle.remove(pipe);
        }
        TimeoutAction::Drop
    });
    match timer {
        Ok(timer) => state.copy.sources = Some((pipe, timer)),
        Err(_) => state.handle.remove(pipe),
    }
}

/// Sends the text to the holder, if it still holds control at the copy's epoch.
fn deliver(state: &mut State, conn: u64, epoch: u32, text: Vec<u8>) {
    if !state.lease.holds(conn, epoch) || text.is_empty() {
        return;
    }
    let Ok(text) = String::from_utf8(text) else {
        tracing::info!("the browser copied text that is not UTF-8; not sent");
        return;
    };
    let message = Message::Copied { lease_epoch: epoch, text };
    if state.hub.viewers.send_to(conn, &message) {
        return;
    }
    // No room in the holder's queue now (media is catching up): try again shortly, while the lease stays the same.
    let started = Instant::now();
    let retry = state.handle.insert_source(Timer::from_duration(RETRY), move |_, _, data: &mut Data| {
        let state = &mut data.state;
        let current = state.lease.holds(conn, epoch);
        if current && state.hub.viewers.send_to(conn, &message) {
            state.copy.retry = None;
            return TimeoutAction::Drop;
        }
        if !current || started.elapsed() >= RETRY_FOR {
            tracing::info!(conn, "the copied text could not go to the holder");
            state.copy.retry = None;
            return TimeoutAction::Drop;
        }
        TimeoutAction::ToDuration(RETRY)
    });
    state.copy.retry = retry.ok();
}
