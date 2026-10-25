//! Files the person drags onto the screen: once their uploads are stored, the display drags them into the page itself,
//! as a compositor-owned drag offering `text/uri-list`, at the point the person dropped them.
//!
//! The drag enters the surface under that point and waits (at most [`ACCEPT_WITHIN`]) for the page to accept the type
//! and an action, as a person's hand would hover; then it drops. A page that does not accept gets the drag cancelled,
//! and a lease change cancels it too. Only paths of uploads the launcher stored for this holder are ever offered.
//!
//! Before that, while the person still drags the files over the screen, the display drags placeholders of the same
//! media types over the page ([`drag_files`]): the page sees files of those types enter, move, and leave, so it shows
//! its drag-over state, as a page does under a person's hand. Chrome reads a drag's paths when it enters, so these are
//! paths where nothing exists; the page sees no path before a drop, and the hover ends before any drop (input cannot
//! release it), the real drop replacing it with the stored uploads.
//!
//! One drop runs at a time: a drop asked for meanwhile waits its turn, and the latest drag over the page asked for
//! after the last waiting drop resumes once no drop remains (a drag asked for before a drop belonged to that drop or
//! an older one). Waiting drops are bounded by the uploads one lease remembers: the client drops each upload once, so
//! only a client that drops the same uploads again can reach the bound, and its drop is then refused.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use calloop::RegistrationToken;
use calloop::timer::{TimeoutAction, Timer};
use smithay::input::dnd::{DnDGrab, DndAction, Source, SourceMetadata};
use smithay::input::pointer::{Focus, GrabStartData, MotionEvent};
use smithay::utils::{IsAlive, Serial};

use crate::state::{Data, State};

/// How long the page has to accept the drag before it is cancelled.
const ACCEPT_WITHIN: Duration = Duration::from_secs(2);
/// How long the drag moves in place before it drops, from its first motion after the dragged paths were handed to
/// Chrome. Chrome accepts a drag as it enters, before it has the drag's data, which it reads in the background, and
/// shows the page the drag only once it has; a drop before that reaches no page, and Chrome opens the files it reads
/// afterwards in tabs of their own. It learns the page's answer only as the pointer moves. A margin, not a proof:
/// Chrome says neither when it has read the paths nor when the page has answered.
const SETTLE: Duration = Duration::from_millis(60);
/// How often acceptance is checked.
const CHECK: Duration = Duration::from_millis(20);
const URI_LIST: &str = "text/uri-list";
/// Where the placeholders of a hover point: a directory the launcher never creates.
const PLACEHOLDERS: &str = "/uploads/dragging";
/// Most drops that wait for the one in progress: as many as the uploads a lease remembers.
const MAX_QUEUED: usize = crate::uploads::MAX_UPLOADS;

/// What Chrome answered so far, and when the dragged paths were handed to it (written whole, the pipe closed).
#[derive(Default)]
struct Answer {
    accepted: bool,
    action: bool,
    written_at: Option<Instant>,
    done: bool,
}

/// The dragged files, as the compositor offers them.
struct Files {
    uri_list: Arc<str>,
    answer: Arc<Mutex<Answer>>,
}

impl IsAlive for Files {
    fn alive(&self) -> bool {
        !self.answer.lock().unwrap().done
    }
}

impl Source for Files {
    fn metadata(&self) -> Option<SourceMetadata> {
        Some(SourceMetadata {
            mime_types: vec![URI_LIST.to_owned()],
            dnd_actions: [DndAction::Copy].into_iter().collect(),
        })
    }

    fn accepted(&self, mime_type: Option<String>) {
        self.answer.lock().unwrap().accepted = mime_type.as_deref() == Some(URI_LIST);
    }

    fn choose_action(&self, action: DndAction) {
        self.answer.lock().unwrap().action = action == DndAction::Copy;
    }

    fn send(&self, mime_type: &str, fd: std::os::fd::OwnedFd) {
        if mime_type == URI_LIST {
            let answer = self.answer.clone();
            crate::state::write_later(fd, self.uri_list.clone(), move || {
                answer.lock().unwrap().written_at = Some(Instant::now());
            });
        }
    }

    fn drop_performed(&self) {}

    fn cancel(&self) {
        self.answer.lock().unwrap().done = true;
    }

    fn finished(&self) {
        self.answer.lock().unwrap().done = true;
    }
}

/// A `file://` URI per path, every byte outside the unreserved set and `/` percent-encoded, one per line.
fn uri_list(paths: &[String]) -> String {
    let mut list = String::new();
    for path in paths {
        list.push_str("file://");
        for &b in path.as_bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
                list.push(b as char);
            } else {
                list.push_str(&format!("%{b:02X}"));
            }
        }
        list.push_str("\r\n");
    }
    list
}

/// The placeholders of the files the person drags over the page, by media type, and the serial of their grab.
pub struct Hover {
    types: Vec<String>,
    serial: Serial,
}

/// The drop in progress: its acceptance check and the serial of its grab.
pub struct Dropping {
    check: RegistrationToken,
    serial: Serial,
}

/// A drop or a drag over the page asked for while a drop was in progress.
pub enum Waiting {
    Drop { conn: u64, epoch: u32, x: u16, y: u16, uploads: Vec<u32> },
    Drag { conn: u64, epoch: u32, x: u16, y: u16, types: Vec<String> },
}

/// True when the pointer's grab is still the one started with `serial` (and not one that followed it).
fn grabbed(state: &State, serial: Serial) -> bool {
    state.seat.get_pointer().is_some_and(|p| p.has_grab(serial))
}

/// True while the person's files are dragged over the page.
pub fn hovering(state: &State) -> bool {
    state.uploads.hover.as_ref().is_some_and(|h| grabbed(state, h.serial))
}

/// True while the person's files are dragged over the page or dropped on it: no other pointer input then.
pub fn dragging_files(state: &State) -> bool {
    hovering(state) || state.uploads.drag.as_ref().is_some_and(|d| grabbed(state, d.serial))
}

/// Whether a drop may go: Chrome accepts it, and the drag first moved in place after its paths were written
/// [`SETTLE`] ago.
fn ready(accepted: bool, moved_at: Option<Instant>, now: Instant) -> bool {
    accepted && moved_at.is_some_and(|moved| now.saturating_duration_since(moved) >= SETTLE)
}

/// Ends the grab started with `serial`, if it is still the pointer's.
fn ungrab(state: &mut State, serial: Serial) {
    if grabbed(state, serial)
        && let Some(pointer) = state.seat.get_pointer()
    {
        pointer.unset_grab(state, state.next_serial(), smithay::backend::input::InputTime::now());
    }
}

/// Ends the files' hover, if any: the drag leaves the page.
fn end_hover(state: &mut State) {
    if let Some(hover) = state.uploads.hover.take() {
        ungrab(state, hover.serial);
    }
}

/// Ends the hover, the drop in progress, and everything waiting for it, at once: a lease change must not let a later
/// release complete them.
pub fn cancel(state: &mut State) {
    state.uploads.waiting.clear();
    end_hover(state);
    if let Some(dropping) = state.uploads.drag.take() {
        state.handle.remove(dropping.check);
        ungrab(state, dropping.serial);
    }
}

/// Starts a compositor drag of `files` at (`x`, `y`), held to the output: it enters the surface under that point.
/// Returns its grab's serial.
fn start_drag(state: &mut State, x: f64, y: f64, files: Files) -> Option<Serial> {
    let pointer = state.seat.get_pointer()?;
    let (x, y) = crate::input::clamp_to_output(x, y);
    crate::input::move_pointer(state, x, y);
    let focus = crate::input::surface_under(state, x, y);
    let start = GrabStartData { focus: focus.clone(), button: crate::input::BTN_LEFT, location: (x, y).into() };
    let grab = DnDGrab::new_pointer(&state.dh, start, files, state.seat.clone());
    let serial = state.next_serial();
    pointer.set_grab(state, grab, serial, Focus::Keep);
    let motion = MotionEvent {
        location: (x, y).into(),
        serial: state.next_serial(),
        time: smithay::backend::input::InputTime::now(),
    };
    pointer.motion(state, focus, &motion);
    pointer.frame(state);
    Some(serial)
}

/// The holder `conn` at `epoch` drags files of `types` over (`x`, `y`): the placeholders' drag starts there, or
/// follows. While a drop is in progress, the latest such drag waits for it.
pub fn drag_files(state: &mut State, conn: u64, epoch: u32, x: u16, y: u16, types: Vec<String>) {
    if state.uploads.drag.is_some() {
        state.uploads.waiting.retain(|w| matches!(w, Waiting::Drop { .. }));
        state.uploads.waiting.push_back(Waiting::Drag { conn, epoch, x, y, types });
        return;
    }
    let (x, y) = (f64::from(x), f64::from(y));
    if hovering(state) && state.uploads.hover.as_ref().is_some_and(|h| h.types == types) {
        crate::input::move_pointer(state, x, y);
        return;
    }
    // The uploads and their drags are the holder's from here on: a first upload during the hover keeps it.
    crate::uploads::own(state, conn, epoch);
    end_hover(state);
    let paths: Vec<String> =
        types.iter().enumerate().map(|(i, t)| format!("{PLACEHOLDERS}/{}{}", i + 1, extension(t))).collect();
    let files = Files { uri_list: Arc::from(uri_list(&paths)), answer: Arc::new(Mutex::new(Answer::default())) };
    if let Some(serial) = start_drag(state, x, y, files) {
        state.uploads.hover = Some(Hover { types, serial });
    }
}

/// The file name extension Chrome gives `media_type` back from, for the media types pages commonly accept; none for
/// the rest (the page then sees an empty type, as for an unknown file).
fn extension(media_type: &str) -> &'static str {
    match media_type {
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/gif" => ".gif",
        "image/webp" => ".webp",
        "image/avif" => ".avif",
        "image/svg+xml" => ".svg",
        "image/bmp" => ".bmp",
        "image/x-icon" | "image/vnd.microsoft.icon" => ".ico",
        "image/tiff" => ".tiff",
        "image/heic" => ".heic",
        "application/pdf" => ".pdf",
        "application/json" => ".json",
        "application/zip" => ".zip",
        "application/gzip" => ".gz",
        "application/msword" => ".doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => ".docx",
        "application/vnd.ms-excel" => ".xls",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => ".xlsx",
        "application/vnd.ms-powerpoint" => ".ppt",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => ".pptx",
        "text/plain" => ".txt",
        "text/csv" => ".csv",
        "text/html" => ".html",
        "text/markdown" => ".md",
        "text/xml" | "application/xml" => ".xml",
        "audio/mpeg" => ".mp3",
        "audio/wav" | "audio/x-wav" => ".wav",
        "audio/ogg" => ".ogg",
        "video/mp4" => ".mp4",
        "video/webm" => ".webm",
        "video/quicktime" => ".mov",
        _ => "",
    }
}

/// The person's files of the drag over the page ended: it leaves, and no drag waits to resume.
pub fn drag_ended(state: &mut State) {
    state.uploads.waiting.retain(|w| matches!(w, Waiting::Drop { .. }));
    end_hover(state);
}

/// Runs what waited for the drop that just ended: the next drop, else the latest drag over the page.
fn next(state: &mut State) {
    match state.uploads.waiting.pop_front() {
        Some(Waiting::Drop { conn, epoch, x, y, uploads }) => drop_files(state, conn, epoch, x, y, uploads),
        Some(Waiting::Drag { conn, epoch, x, y, types }) => drag_files(state, conn, epoch, x, y, types),
        None => {}
    }
}

/// Drags the stored uploads `uploads` of the holder `conn` at `epoch` onto the page at (`x`, `y`), after the drop in
/// progress, if any.
pub fn drop_files(state: &mut State, conn: u64, epoch: u32, x: u16, y: u16, uploads: Vec<u32>) {
    if state.uploads.drag.is_some() {
        state.uploads.waiting.retain(|w| matches!(w, Waiting::Drop { .. }));
        if state.uploads.waiting.len() < MAX_QUEUED {
            state.uploads.waiting.push_back(Waiting::Drop { conn, epoch, x, y, uploads });
        } else {
            tracing::warn!(conn, "refused a drop beyond the waiting bound");
        }
        return;
    }
    // The hover ends first, also when the uploads are not the holder's: it never outlives the drop.
    end_hover(state);
    let Some(paths) = crate::uploads::stored_paths(state, conn, epoch, &uploads) else { return next(state) };
    let (x, y) = (f64::from(x), f64::from(y));
    let answer = Arc::new(Mutex::new(Answer::default()));
    let files = Files { uri_list: Arc::from(uri_list(&paths)), answer: answer.clone() };
    let Some(serial) = start_drag(state, x, y, files) else { return next(state) };
    let started = Instant::now();
    let mut released = false;
    let mut moved_at = None;
    let checked = state.handle.insert_source(Timer::from_duration(CHECK), move |_, _, data: &mut Data| {
        let state = &mut data.state;
        let current = state.lease.holds(conn, epoch);
        let (accepted, written, gone) = {
            let answer = answer.lock().unwrap();
            (answer.accepted && answer.action, answer.written_at.is_some(), answer.done)
        };
        let held = grabbed(state, serial);
        if gone || (released && !held) {
            state.uploads.drag = None;
            next(state);
            return TimeoutAction::Drop;
        }
        let late = started.elapsed() >= if released { 2 * ACCEPT_WITHIN } else { ACCEPT_WITHIN };
        if !current || late {
            state.uploads.drag = None;
            ungrab(state, serial);
            next(state);
            return TimeoutAction::Drop;
        }
        // Released over the page once Chrome has had the dragged paths and heard the page's answer: the drop. The
        // check stays until the grab has ended, so a lease change before that still cancels it.
        if !released && ready(accepted, moved_at, Instant::now()) {
            released = true;
            crate::input::press(state, crate::input::BTN_LEFT, false);
        } else if !released && written {
            // In place: each motion has Chrome ask the page again.
            crate::input::move_pointer(state, x, y);
            moved_at.get_or_insert_with(Instant::now);
        }
        TimeoutAction::ToDuration(CHECK)
    });
    match checked {
        Ok(check) => state.uploads.drag = Some(Dropping { check, serial }),
        Err(_) => {
            ungrab(state, serial);
            next(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_waits_for_motion_after_chrome_has_the_paths_and_for_acceptance() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        // Accepted as it enters, before the drag has moved with the data handed over (however late): no drop yet.
        assert!(!ready(true, None, at(20)));
        assert!(!ready(true, None, at(5000)));
        assert!(!ready(true, Some(at(30)), at(40)));
        assert!(ready(true, Some(at(30)), at(90)));
        assert!(!ready(false, Some(at(30)), at(100)));
    }

    #[test]
    fn uri_lists_encode_paths_one_per_line() {
        let list = uri_list(&["/uploads/1/a b.png".into(), "/uploads/2/relatório.pdf".into()]);
        assert_eq!(list, "file:///uploads/1/a%20b.png\r\nfile:///uploads/2/relat%C3%B3rio.pdf\r\n");
    }

    #[test]
    fn every_extension_is_one_the_media_type_comes_back_from() {
        for t in ["image/png", "application/pdf", "text/plain", "video/mp4"] {
            assert!(extension(t).starts_with('.') && extension(t).len() > 1, "{t}");
        }
        assert_eq!(extension(""), "");
        assert_eq!(extension("application/x-unknown"), "");
    }
}
