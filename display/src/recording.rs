//! Recording the session for the lease holder (protocol `RecordStart`, `RecordCamera`, `RecordMicrophone`,
//! `RecordStop`): one recording at a time, composed and encoded by a worker of its own (`stream::record`) and offered
//! to the holder as a download that grows as it is made (`files::start_recording`), ended with its size once the worker
//! is done. The lease moving, the download going (cancelled, never read, failed), or the holder leaving ends it; only a
//! recording the worker finished and whose download is still there ends well.

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex, PoisonError};

use display_protocol::{CameraLayout, Message, RecordPhase};

use crate::state::{Data, State};
use crate::stream::record::{Finished, Input, Picture, Settings, Worker};

/// Where the page's sound goes while a recording runs: the audio thread hands each packet's samples through it.
pub type Tap = Arc<Mutex<Option<SyncSender<Input>>>>;

#[derive(Default)]
pub struct Recordings {
    current: Option<Recording>,
    pub tap: Tap,
}

struct Recording {
    conn: u64,
    lease_epoch: u32,
    id: u32,
    file: u32,
    worker: Worker,
    /// Whether the start announced the camera and the microphone: the recording takes only what it announced.
    camera: bool,
    microphone: bool,
    /// Asked to stop: it takes nothing new.
    stopping: bool,
    /// The download went: whatever the worker says, the recording failed.
    lost: bool,
}

/// What the holder asked a recording to be.
pub struct Request {
    pub id: u32,
    pub layout: CameraLayout,
    pub camera: bool,
    pub microphone: bool,
    pub page_sound: bool,
}

fn tell(
    state: &State,
    conn: u64,
    lease_epoch: u32,
    id: u32,
    phase: RecordPhase,
    file: u32,
    finished: Option<&Finished>,
) {
    let (seconds, bytes) = finished.map_or((0, 0), |f| (f.seconds, f.bytes));
    let message = Message::RecordState { lease_epoch, recording: id, phase, file, seconds, bytes };
    state.hub.viewers.send_control_to(conn, &message);
}

/// Starts a recording for the holder `conn` (the lease was checked), or says it failed.
pub fn start(state: &mut State, conn: u64, request: Request) {
    let lease_epoch = state.lease.epoch;
    let id = request.id;
    if state.recordings.current.is_some() {
        tracing::info!(conn, "a second recording was refused");
        return tell(state, conn, lease_epoch, id, RecordPhase::Failed, 0, None);
    }
    let (reader, writer) = match UnixStream::pair().and_then(|(r, w)| r.set_nonblocking(true).map(|()| (r, w))) {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(%e, "no socket for a recording");
            return tell(state, conn, lease_epoch, id, RecordPhase::Failed, 0, None);
        }
    };
    let Some(file) = crate::files::start_recording(state, OwnedFd::from(reader), file_name()) else {
        return tell(state, conn, lease_epoch, id, RecordPhase::Failed, 0, None);
    };
    // The worker's end comes back on the compositor's loop.
    let (done, ended) = calloop::channel::channel::<Finished>();
    let source = state.handle.insert_source(ended, move |event, _, data: &mut Data| {
        if let calloop::channel::Event::Msg(finished) = event {
            finish(&mut data.state, id, finished);
            let _ = data.display.flush_clients();
        }
    });
    if source.is_err() {
        crate::files::fail_recording(state, file);
        return tell(state, conn, lease_epoch, id, RecordPhase::Failed, file, None);
    }
    let settings =
        Settings { width: state.scene.width as u32, height: state.scene.height as u32, layout: request.layout };
    let first = state.scene.framebuffer.clone();
    let pointer = crate::stream::pointer_now(state);
    let worker = match Worker::start(settings, first, pointer, writer, move |finished| {
        let _ = done.send(finished);
    }) {
        Ok(worker) => worker,
        Err(why) => {
            tracing::warn!(%why, "a recording did not start");
            crate::files::fail_recording(state, file);
            return tell(state, conn, lease_epoch, id, RecordPhase::Failed, file, None);
        }
    };
    // The page's sound reaches the recording only when it asked for it: otherwise the audio thread never queues it.
    if request.page_sound {
        *state.recordings.tap.lock().unwrap_or_else(PoisonError::into_inner) = Some(worker.sender());
    }
    state.recordings.current = Some(Recording {
        conn,
        lease_epoch,
        id,
        file,
        worker,
        camera: request.camera,
        microphone: request.microphone,
        stopping: false,
        lost: false,
    });
    tracing::info!(conn, file, "recording started");
    tell(state, conn, lease_epoch, id, RecordPhase::Recording, file, None);
}

/// The recording `id` of `conn`, while it runs.
fn current(state: &mut State, conn: u64, id: u32) -> Option<&mut Recording> {
    state.recordings.current.as_mut().filter(|r| r.conn == conn && r.id == id && !r.stopping)
}

pub fn camera(state: &mut State, conn: u64, id: u32, timestamp_us: u64, jpeg: Vec<u8>) {
    if let Some(r) = current(state, conn, id).filter(|r| r.camera) {
        r.worker.send(Input::Camera { timestamp_us, jpeg });
    }
}

pub fn microphone(state: &mut State, conn: u64, id: u32, timestamp_us: u64, packet: Vec<u8>) {
    if let Some(r) = current(state, conn, id).filter(|r| r.microphone) {
        r.worker.send(Input::Microphone { timestamp_us, packet });
    }
}

/// The camera moved or changed its look.
pub fn layout(state: &mut State, conn: u64, id: u32, layout: CameraLayout) {
    if let Some(r) = current(state, conn, id) {
        r.worker.layout(layout);
    }
}

pub fn stop(state: &mut State, conn: u64, id: u32) {
    if current(state, conn, id).is_some() {
        tracing::info!(conn, "recording stopping");
        end(state, false);
    }
}

/// Each composed frame: the worker takes the newest.
pub fn frame(state: &State) {
    if let Some(r) = &state.recordings.current {
        r.worker.frame(state.scene.framebuffer.clone());
    }
}

/// The pointer moved to `at`.
pub fn pointer_moved(state: &State, at: (u16, u16)) {
    if let Some(r) = &state.recordings.current {
        r.worker.pointer_at(at);
    }
}

/// The pointer's picture changed (none: hidden); `picture` makes it, only while a recording runs.
pub fn pointer_changed(state: &State, picture: impl FnOnce() -> Option<Arc<Picture>>) {
    if let Some(r) = &state.recordings.current {
        r.worker.pointer_picture(picture());
    }
}

fn end(state: &mut State, lost: bool) {
    let Some(r) = &mut state.recordings.current else { return };
    r.lost |= lost;
    r.stopping = true;
    r.worker.stop();
}

/// The lease moved: the recording ends (its download already went with the lease).
pub fn lease_changed(state: &mut State) {
    end(state, true);
}

/// A recording's download went: the recording ends, failed.
pub fn download_gone(state: &mut State, file: u32) {
    if state.recordings.current.as_ref().is_some_and(|r| r.file == file) {
        end(state, true);
    }
}

/// The worker finished: the download ends with the recording's size (or goes), and the holder is told.
fn finish(state: &mut State, id: u32, finished: Finished) {
    let Some(r) = state.recordings.current.take_if(|r| r.id == id) else { return };
    *state.recordings.tap.lock().unwrap_or_else(PoisonError::into_inner) = None;
    let ok = finished.failure.is_none() && !r.lost;
    if ok {
        crate::files::end_recording(state, r.file, finished.bytes);
    } else {
        crate::files::fail_recording(state, r.file);
    }
    tracing::info!(file = r.file, bytes = finished.bytes, seconds = finished.seconds, failure = ?finished.failure, lost = r.lost, "recording ended");
    if state.lease.holds(r.conn, r.lease_epoch) {
        let phase = if ok { RecordPhase::Ended } else { RecordPhase::Failed };
        tell(state, r.conn, r.lease_epoch, r.id, phase, r.file, Some(&finished));
    }
}

/// The recording's file name, after the moment it starts (UTC): `easi-run-YYYYMMDD-HHMMSS.webm`.
fn file_name() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil(days as i64);
    format!("easi-run-{y:04}{m:02}{d:02}-{:02}{:02}{:02}.webm", rest / 3600, rest / 60 % 60, rest % 60)
}

/// The proleptic Gregorian date of `days` since 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_follow_the_calendar() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_384), (2025, 10, 23));
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert!(file_name().starts_with("easi-run-") && file_name().ends_with(".webm"));
    }
}
