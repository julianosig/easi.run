//! Files the person gives the page: for the page's file chooser, or dropped on it.
//!
//! The person holding control sends an upload in order, at most [`UPLOAD_WINDOW`] bytes ahead of the display's last
//! acknowledgement; the display passes the bytes through a pipe to the browser's launcher (over `easi_files_v1`),
//! which stores them in its own private directory and answers with the path. The display holds at most a window per
//! upload and [`MAX_ACTIVE`] uploads at once, never a file. A file chooser the page opens is answered with the stored
//! paths the person chose, or cancelled. Everything belongs to one connection and lease epoch: a lease change cancels
//! the chooser and abandons every upload (the launcher drops a file that did not arrive whole). An upload the display
//! forgets whose path the page was never given (chosen, or dropped) can no longer be reached: the launcher removes it,
//! as it does an upload that failed. One the page was given it may still read, so the launcher keeps it until newer
//! uploads need its room: what the launcher stores stays within [`MAX_UPLOADS`] files and [`UPLOAD_ROOM`] bytes, newer
//! uploads taking the room of older ones in [`Uploads::room`]'s order (those a page was given first).

use std::collections::VecDeque;
use std::os::fd::{AsFd, OwnedFd};

use std::time::{Duration, Instant};

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{Interest, Mode, PostAction, RegistrationToken};
use display_protocol::browser::UPLOAD_ROOM;
use display_protocol::{MAX_CHOSEN, MAX_UPLOAD, Message, UPLOAD_WINDOW};
use rustix::pipe::PipeFlags;

use crate::files::Watch;
use crate::files::protocol::easi_files_v1::EasiFilesV1;
use crate::state::{Data, State};

/// Uploads being received at once.
const MAX_ACTIVE: usize = 4;
/// Uploads kept at once: those of the lease (received or stored) and the forgotten ones a page was given. Past it,
/// the oldest finished one nothing waits for goes.
pub const MAX_UPLOADS: usize = 4 * MAX_CHOSEN;
/// Bytes between two acknowledgements.
const ACK_EVERY: u64 = 64 * 1024;
/// An upload with no progress for this long is abandoned, so it cannot hold a slot (or the launcher's reader) forever.
const STALLED: Duration = Duration::from_secs(30);
/// How often stalled uploads are looked for, while any is being received.
const SWEEP: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq)]
enum Stage {
    Receiving,
    /// All bytes went into the pipe; the launcher has not answered yet.
    Sent,
    Stored(String),
    Failed,
}

struct Upload {
    /// The id the launcher knows it by.
    id: u32,
    /// The connection's own id for it.
    client: u32,
    size: u64,
    /// Bytes the person sent.
    received: u64,
    /// Bytes written into the pipe, and the last count acknowledged.
    written: u64,
    acked: u64,
    buffer: VecDeque<u8>,
    /// When the person last sent a piece, or the pipe last took one.
    progress: Instant,
    /// The pipe's write end, and the loop source that writes it when it has room.
    pipe: Option<OwnedFd>,
    writer: Option<Watch>,
    stage: Stage,
    /// Whether the page was given its path: then it may read the file for as long as the browser runs.
    given: bool,
}

impl Upload {
    fn new(id: u32, client: u32, size: u64, pipe: Option<OwnedFd>) -> Upload {
        Upload {
            id,
            client,
            size,
            received: 0,
            written: 0,
            acked: 0,
            buffer: VecDeque::new(),
            progress: Instant::now(),
            pipe,
            writer: None,
            stage: Stage::Receiving,
            given: false,
        }
    }
}

/// The page's open file chooser.
struct Chooser {
    /// The launcher's id for it, and the one the person's client answers.
    launcher: u32,
    request: u32,
    /// What the person chose, once they did.
    chosen: Option<Vec<u32>>,
}

/// The uploads and chooser of the current holder.
#[derive(Default)]
pub struct Uploads {
    /// The holder connection and lease epoch they belong to.
    owner: Option<(u64, u32)>,
    uploads: Vec<Upload>,
    chooser: Option<Chooser>,
    /// The drop in progress (see [`crate::drop`]).
    pub drag: Option<crate::drop::Dropping>,
    /// The files the person drags over the page before dropping them (see [`crate::drop`]).
    pub hover: Option<crate::drop::Hover>,
    /// The drops and the drag over the page asked for while a drop was in progress, in order.
    pub waiting: std::collections::VecDeque<crate::drop::Waiting>,
    /// The timer that abandons stalled uploads, while any is being received.
    sweep: Option<RegistrationToken>,
    /// The uploads forgotten whose path a page was given, oldest first, by the launcher's id and with their size: the
    /// page may still read them, so the launcher keeps them until their room is needed.
    given: VecDeque<(u32, u64)>,
    /// The launcher's ids of the uploads it is to remove, until it is told to: those forgotten whose path the page was
    /// never given, and those whose room a newer upload needs.
    removing: Vec<u32>,
    /// The holder's ids of the stored uploads forgotten for a newer one's room, until the holder is told.
    gone: Vec<u32>,
    next_id: u32,
    next_request: u32,
}

impl Uploads {
    /// Whether the chooser's choice, the drop in progress, a queued drop, or `held` (the drops waiting behind typed
    /// text) names the holder's upload `client`.
    fn named(&self, client: u32, held: &[u32]) -> bool {
        let chosen = self.chooser.as_ref().and_then(|c| c.chosen.as_deref()).unwrap_or_default();
        let dropping = self.drag.as_ref().map(|d| d.uploads.as_slice()).unwrap_or_default();
        held.contains(&client)
            || chosen.contains(&client)
            || dropping.contains(&client)
            || self
                .waiting
                .iter()
                .any(|w| matches!(w, crate::drop::Waiting::Drop { uploads, .. } if uploads.contains(&client)))
    }

    /// The upload to forget next for room, among those nothing names ([`Uploads::named`]): one that failed (it holds
    /// nothing), then the oldest stored that a page was given, then the oldest stored: one the holder sent and has not
    /// used yet goes last, as it may be about to.
    fn forgettable(&self, held: &[u32]) -> Option<usize> {
        let free = |u: &Upload| !self.named(u.client, held);
        let stored = |u: &Upload| matches!(u.stage, Stage::Stored(_));
        (self.uploads.iter().position(|u| u.stage == Stage::Failed && free(u)))
            .or_else(|| self.uploads.iter().position(|u| stored(u) && u.given && free(u)))
            .or_else(|| self.uploads.iter().position(|u| stored(u) && free(u)))
    }

    /// Bytes the launcher stores or is about to: the uploads known but the failed (removed as they fail), each at its
    /// whole size, and the forgotten ones a page was given.
    fn bytes(&self) -> u64 {
        let known: u64 = self.uploads.iter().filter(|u| u.stage != Stage::Failed).map(|u| u.size).sum();
        known + self.given.iter().map(|(_, size)| size).sum::<u64>()
    }

    /// Makes room for one more upload, of `size` bytes: whether there is, and nothing is forgotten when there is not.
    /// Past [`MAX_UPLOADS`] files or [`UPLOAD_ROOM`] bytes the oldest go: first the forgotten ones a page was given,
    /// then the holder's, in [`Uploads::forgettable`]'s order.
    fn room(&mut self, size: u64, held: &[u32]) -> bool {
        // What cannot go: an upload still arriving, and one that is named.
        let stays = |u: &&Upload| matches!(u.stage, Stage::Receiving | Stage::Sent) || self.named(u.client, held);
        let staying = self.uploads.iter().filter(stays).count();
        let bytes: u64 = self.uploads.iter().filter(stays).filter(|u| u.stage != Stage::Failed).map(|u| u.size).sum();
        if staying >= MAX_UPLOADS || bytes.saturating_add(size) > UPLOAD_ROOM {
            return false;
        }
        while self.uploads.len() + self.given.len() >= MAX_UPLOADS || self.bytes() + size > UPLOAD_ROOM {
            if let Some((id, _)) = self.given.pop_front() {
                self.removing.push(id);
                continue;
            }
            let Some(index) = self.forgettable(held) else { return false };
            let upload = self.uploads.remove(index);
            // A failed one's file went as it failed.
            if matches!(upload.stage, Stage::Stored(_)) {
                self.gone.push(upload.client);
                self.removing.push(upload.id);
            }
        }
        true
    }

    /// Notes what becomes of the uploads a lease's end forgets: one whose path the page was never given the launcher is
    /// to remove; one it was given is kept for the page, until a newer upload needs its room ([`Uploads::room`],
    /// which keeps every upload, forgotten or not, within [`MAX_UPLOADS`]).
    fn forgotten(&mut self, uploads: impl IntoIterator<Item = Upload>) {
        for upload in uploads {
            if upload.given && matches!(upload.stage, Stage::Stored(_)) {
                self.given.push_back((upload.id, upload.size));
            } else {
                self.removing.push(upload.id);
            }
        }
    }

    /// The stored paths of `clients`, if every one is stored.
    fn paths(&self, clients: &[u32]) -> Option<Vec<String>> {
        clients
            .iter()
            .map(|client| match self.uploads.iter().find(|u| u.client == *client).map(|u| &u.stage) {
                Some(Stage::Stored(path)) => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    /// Notes that the page was given the paths of `clients`.
    fn give(&mut self, clients: &[u32]) {
        for upload in self.uploads.iter_mut().filter(|u| clients.contains(&u.client)) {
            upload.given = true;
        }
    }
}

fn launcher(state: &State) -> Option<EasiFilesV1> {
    state.files.launcher.clone()
}

/// Has the launcher remove the uploads noted for it: those nothing can reach, and those whose room is needed. Without
/// a launcher its files are gone with it, those a page was given too.
fn remove_forgotten(state: &mut State) {
    let removing = std::mem::take(&mut state.uploads.removing);
    match launcher(state) {
        Some(launcher) => removing.into_iter().for_each(|id| launcher.remove_upload(id)),
        None => state.uploads.given.clear(),
    }
}

/// Ends everything of the previous holder: a lease change, or a launcher that went away.
pub fn lease_changed(state: &mut State) {
    if let Some(chooser) = state.uploads.chooser.take()
        && let Some(launcher) = launcher(state)
    {
        launcher.chooser_cancelled(chooser.launcher);
    }
    let uploads = std::mem::take(&mut state.uploads.uploads);
    state.uploads.forgotten(uploads);
    state.uploads.gone.clear();
    remove_forgotten(state);
    crate::drop::cancel(state);
    if let Some(sweep) = state.uploads.sweep.take() {
        state.handle.remove(sweep);
    }
    state.uploads.owner = None;
}

/// The page opened a file chooser.
pub fn open_chooser(state: &mut State, launcher_chooser: u32, multiple: bool) {
    let (Some(launcher), Some((conn, epoch))) = (launcher(state), state.lease.current()) else {
        if let Some(launcher) = launcher(state) {
            launcher.chooser_cancelled(launcher_chooser);
        }
        return;
    };
    own(state, conn, epoch);
    if let Some(previous) = state.uploads.chooser.take() {
        launcher.chooser_cancelled(previous.launcher);
        notify(state, Message::FileRequestEnd { lease_epoch: epoch, request: previous.request });
    }
    state.uploads.next_request = state.uploads.next_request.wrapping_add(1).max(1);
    let request = state.uploads.next_request;
    let message = Message::FileRequest { lease_epoch: epoch, request, multiple };
    // The page can open choosers as fast as it likes: one the holder's queue has no room for is cancelled.
    if state.hub.viewers.send_notice_to(conn, &message) {
        state.uploads.chooser = Some(Chooser { launcher: launcher_chooser, request, chosen: None });
    } else {
        launcher.chooser_cancelled(launcher_chooser);
    }
}

/// Makes the uploads the holder's at `epoch`, ending those of anyone before.
pub fn own(state: &mut State, conn: u64, epoch: u32) {
    if state.uploads.owner != Some((conn, epoch)) {
        lease_changed(state);
        state.uploads.owner = Some((conn, epoch));
    }
}

/// A reply to the owner's own uploads.
fn send(state: &State, message: Message) {
    if let Some((conn, _)) = state.uploads.owner {
        state.hub.viewers.send_control_to(conn, &message);
    }
}

/// A notice the owner did not ask for: the page's chooser came or went.
fn notify(state: &State, message: Message) {
    if let Some((conn, _)) = state.uploads.owner {
        state.hub.viewers.send_notice_to(conn, &message);
    }
}

fn done(state: &State, client: u32, stored: bool) {
    if let Some((_, epoch)) = state.uploads.owner {
        send(state, Message::UploadDone { lease_epoch: epoch, upload: client, stored });
    }
}

/// The holder starts an upload (the lease was checked).
pub fn start(state: &mut State, conn: u64, epoch: u32, client: u32, size: u64, name: &str) {
    own(state, conn, epoch);
    let active = state.uploads.uploads.iter().filter(|u| u.stage == Stage::Receiving).count();
    let known = state.uploads.uploads.iter().any(|u| u.client == client);
    let Some(launcher) = launcher(state) else { return done(state, client, false) };
    let held: Vec<u32> = state.typing.dropping().collect();
    if known || size > MAX_UPLOAD || active >= MAX_ACTIVE || !state.uploads.room(size, &held) {
        return done(state, client, false);
    }
    // What was forgotten for this one's room: the launcher removes it first, and the holder hears of each stored
    // upload it can no longer choose.
    remove_forgotten(state);
    for gone in std::mem::take(&mut state.uploads.gone) {
        done(state, gone, false);
    }
    let Ok((read, write)) = rustix::pipe::pipe_with(PipeFlags::CLOEXEC) else { return done(state, client, false) };
    // The display never waits on the pipe; the launcher reads its end blocking.
    let nonblocking = rustix::fs::fcntl_getfl(&write).map(|f| f | rustix::fs::OFlags::NONBLOCK);
    if nonblocking.and_then(|f| rustix::fs::fcntl_setfl(&write, f)).is_err() {
        return done(state, client, false);
    }
    state.uploads.next_id = state.uploads.next_id.wrapping_add(1).max(1);
    let id = state.uploads.next_id;
    launcher.upload(id, read.as_fd(), crate::files::sanitized(name), (size >> 32) as u32, size as u32);
    state.uploads.uploads.push(Upload::new(id, client, size, Some(write)));
    if size == 0 {
        // Nothing to wait for: closing the pipe now stores an empty file.
        let index = state.uploads.uploads.len() - 1;
        flush(state, index);
    }
    sweep_later(state);
}

/// Abandons, every [`SWEEP`], the uploads with no progress for [`STALLED`]; stops once none is in flight.
fn sweep_later(state: &mut State) {
    if state.uploads.sweep.is_some() {
        return;
    }
    let sweep = state.handle.insert_source(Timer::from_duration(SWEEP), |_, _, data: &mut Data| {
        let state = &mut data.state;
        let stalled: Vec<usize> = (0..state.uploads.uploads.len())
            .filter(|&i| {
                let u = &state.uploads.uploads[i];
                matches!(u.stage, Stage::Receiving | Stage::Sent) && u.progress.elapsed() >= STALLED
            })
            .collect();
        for index in stalled {
            fail(state, index);
        }
        if state.uploads.uploads.iter().any(|u| matches!(u.stage, Stage::Receiving | Stage::Sent)) {
            TimeoutAction::ToDuration(SWEEP)
        } else {
            state.uploads.sweep = None;
            TimeoutAction::Drop
        }
    });
    state.uploads.sweep = sweep.ok();
}

/// The next piece of an upload (the lease was checked).
pub fn data(state: &mut State, conn: u64, epoch: u32, client: u32, offset: u64, bytes: &[u8]) {
    if state.uploads.owner != Some((conn, epoch)) {
        return;
    }
    let Some(index) = state.uploads.uploads.iter().position(|u| u.client == client) else { return };
    let upload = &mut state.uploads.uploads[index];
    let in_order = upload.stage == Stage::Receiving && offset == upload.received;
    let within = upload.received + bytes.len() as u64 <= upload.size.min(upload.acked + UPLOAD_WINDOW);
    if !in_order || !within {
        return fail(state, index);
    }
    upload.received += bytes.len() as u64;
    upload.progress = Instant::now();
    upload.buffer.extend(bytes);
    flush(state, index);
}

/// Fails the upload at `index`, once: the launcher may say an upload failed as often as it likes, and the owner hears
/// of each upload's end once.
fn fail(state: &mut State, index: usize) {
    let upload = &mut state.uploads.uploads[index];
    if upload.stage == Stage::Failed {
        return;
    }
    upload.stage = Stage::Failed;
    // Its memory too: an upload is remembered long after its bytes went.
    upload.buffer = VecDeque::new();
    upload.pipe = None;
    upload.writer = None;
    let (id, client) = (upload.id, upload.client);
    // Nothing can choose it any more: whatever the launcher stored of it goes now (once written, if it still is).
    if let Some(launcher) = launcher(state) {
        launcher.remove_upload(id);
    }
    done(state, client, false);
    finish_chooser(state);
}

/// Writes what the pipe takes now, acknowledges, closes the pipe once all of it went, and waits for room otherwise.
fn flush(state: &mut State, index: usize) {
    let upload = &mut state.uploads.uploads[index];
    let Some(pipe) = upload.pipe.as_ref() else { return };
    while !upload.buffer.is_empty() {
        let (front, _) = upload.buffer.as_slices();
        match rustix::io::write(pipe, front) {
            Ok(n) => {
                upload.buffer.drain(..n);
                upload.written += n as u64;
                upload.progress = Instant::now();
            }
            Err(rustix::io::Errno::INTR) => {}
            Err(rustix::io::Errno::AGAIN) => break,
            Err(_) => return fail(state, index),
        }
    }
    let ack = upload.written == upload.size || upload.written - upload.acked >= ACK_EVERY;
    let waiting = !upload.buffer.is_empty();
    if upload.written == upload.size {
        upload.stage = Stage::Sent;
        // Closing the write end tells the launcher the file is whole. Its buffer, empty, gives its memory back.
        upload.pipe = None;
        upload.buffer = VecDeque::new();
    }
    let (id, client, written) = (upload.id, upload.client, upload.written);
    if ack {
        state.uploads.uploads[index].acked = written;
        if let Some((_, epoch)) = state.uploads.owner {
            send(state, Message::UploadAck { lease_epoch: epoch, upload: client, received: written });
        }
    }
    state.uploads.uploads[index].writer = None;
    if waiting {
        wait_for_room(state, index, id);
    }
}

/// Retries the pipe once it has room again.
fn wait_for_room(state: &mut State, index: usize, id: u32) {
    let Some(pipe) = state.uploads.uploads[index].pipe.as_ref() else { return };
    let Ok(watch) = rustix::io::dup(pipe) else { return fail(state, index) };
    let token =
        state.handle.insert_source(Generic::new(watch, Interest::WRITE, Mode::Level), move |_, _, data: &mut Data| {
            let state = &mut data.state;
            if let Some(index) = state.uploads.uploads.iter().position(|u| u.id == id) {
                // `flush` adds a new source if it must wait again; this one ends here.
                state.uploads.uploads[index].writer = None;
                flush(state, index);
            }
            Ok(PostAction::Remove)
        });
    match token {
        Ok(token) => state.uploads.uploads[index].writer = Some(Watch::new(&state.handle, token)),
        Err(_) => fail(state, index),
    }
}

/// Chromium closed the chooser `launcher_chooser`: the person is no longer asked.
pub fn close_chooser(state: &mut State, launcher_chooser: u32) {
    if state.uploads.chooser.as_ref().is_some_and(|c| c.launcher == launcher_chooser)
        && let Some(chooser) = state.uploads.chooser.take()
        && let Some((_, epoch)) = state.uploads.owner
    {
        notify(state, Message::FileRequestEnd { lease_epoch: epoch, request: chooser.request });
    }
}

/// The launcher stored upload `id` at `path`.
pub fn stored(state: &mut State, id: u32, path: String) {
    let Some(index) = state.uploads.uploads.iter().position(|u| u.id == id) else { return };
    let valid = path.strip_prefix(display_protocol::browser::UPLOAD_DIR).is_some_and(|rest| rest.starts_with('/'))
        && path.len() <= 4096
        && !path.contains('\0')
        && !path.split('/').any(|part| part == ".." || part == ".");
    if state.uploads.uploads[index].stage != Stage::Sent || !valid {
        return fail(state, index);
    }
    state.uploads.uploads[index].stage = Stage::Stored(path);
    done(state, state.uploads.uploads[index].client, true);
    finish_chooser(state);
}

/// The launcher could not store upload `id`.
pub fn failed(state: &mut State, id: u32) {
    if let Some(index) = state.uploads.uploads.iter().position(|u| u.id == id) {
        fail(state, index);
    }
}

/// The person chose uploads for the chooser `request` (none cancels it).
pub fn choose(state: &mut State, conn: u64, epoch: u32, request: u32, uploads: Vec<u32>) {
    if state.uploads.owner != Some((conn, epoch)) {
        return;
    }
    let Some(chooser) = state.uploads.chooser.as_mut() else { return };
    if chooser.request != request || chooser.chosen.is_some() {
        return;
    }
    chooser.chosen = Some(uploads);
    finish_chooser(state);
}

/// Answers the chooser once every chosen upload is stored, or cancels it if one failed or none was chosen.
fn finish_chooser(state: &mut State) {
    let Some(chooser) = state.uploads.chooser.as_ref() else { return };
    let Some(chosen) = chooser.chosen.as_ref() else { return };
    let mut cancelled = chosen.is_empty();
    for client in chosen {
        match state.uploads.uploads.iter().find(|u| u.client == *client).map(|u| &u.stage) {
            Some(Stage::Stored(_)) => {}
            // Still arriving: answered when it is stored.
            Some(Stage::Receiving | Stage::Sent) => return,
            Some(Stage::Failed) | None => {
                cancelled = true;
                break;
            }
        }
    }
    let chooser = state.uploads.chooser.take().expect("checked above");
    let Some(launcher) = launcher(state) else { return };
    let chosen = chooser.chosen.as_deref().unwrap_or_default();
    let paths = if cancelled { None } else { state.uploads.paths(chosen) };
    let Some(paths) = paths else {
        launcher.chooser_cancelled(chooser.launcher);
        return;
    };
    state.uploads.give(chosen);
    for path in paths {
        launcher.chosen(chooser.launcher, path);
    }
    launcher.chooser_done(chooser.launcher);
}

/// The stored paths of `uploads` of the holder `conn` at `epoch`, if every one is stored.
pub fn paths(state: &State, conn: u64, epoch: u32, uploads: &[u32]) -> Option<Vec<String>> {
    if state.uploads.owner != Some((conn, epoch)) {
        return None;
    }
    state.uploads.paths(uploads)
}

/// Notes that the page was given the paths of `uploads`: the launcher keeps them while it has room for them.
pub fn give(state: &mut State, uploads: &[u32]) {
    state.uploads.give(uploads);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload(client: u32, stage: Stage) -> Upload {
        Upload { stage, ..Upload::new(client, client, 0, None) }
    }

    #[test]
    fn past_the_bound_the_oldest_finished_upload_nothing_names_goes() {
        let mut uploads = Uploads::default();
        uploads.uploads.push(upload(1, Stage::Stored("/uploads/1".into())));
        uploads.uploads.push(upload(2, Stage::Receiving));
        uploads.uploads.push(upload(3, Stage::Failed));
        uploads.uploads.push(upload(4, Stage::Stored("/uploads/4".into())));
        uploads.uploads.extend((5..=MAX_UPLOADS as u32).map(|c| upload(c, Stage::Sent)));
        // The chooser's choice and a queued drop keep theirs; one still arriving stays.
        uploads.chooser = Some(Chooser { launcher: 1, request: 1, chosen: Some(vec![1]) });
        uploads.waiting.push_back(crate::drop::Waiting::Drop { conn: 1, epoch: 1, x: 0, y: 0, uploads: vec![3] });
        assert!(uploads.room(0, &[]));
        let clients: Vec<u32> = uploads.uploads.iter().map(|u| u.client).collect();
        assert_eq!(clients[..3], [1, 2, 3], "the stored upload nothing names went");
        assert_eq!(uploads.uploads.len(), MAX_UPLOADS - 1);
        assert_eq!(uploads.removing, [4], "the page never had it, so the launcher removes it");
        assert_eq!(uploads.gone, [4], "and the holder hears it can no longer choose it");
        uploads.uploads.push(upload(1000, Stage::Receiving));
        assert!(!uploads.room(0, &[]), "nothing else may go");
    }

    #[test]
    fn a_drop_in_progress_keeps_its_uploads_until_it_ends() {
        let mut uploads = Uploads::default();
        uploads.uploads.extend((1..=MAX_UPLOADS as u32).map(|c| upload(c, Stage::Stored(format!("/uploads/{c}")))));
        uploads.drag = Some(crate::drop::Dropping::of(vec![1, 2]));
        assert!(uploads.room(0, &[]));
        assert_eq!(uploads.removing, [3], "the oldest upload the drop does not name went");
        uploads.drag = None;
        uploads.uploads.push(upload(1000, Stage::Receiving));
        assert!(uploads.room(0, &[]));
        assert_eq!(uploads.removing, [3, 1], "once the drop ended without giving it, it may go");
    }

    #[test]
    fn a_forgotten_upload_is_removed_only_if_the_page_was_never_given_its_path() {
        let mut uploads = Uploads::default();
        uploads.uploads.push(upload(1, Stage::Stored("/uploads/1/a".into())));
        uploads.uploads.push(upload(2, Stage::Stored("/uploads/2/b".into())));
        uploads.uploads.push(upload(3, Stage::Sent));
        uploads.uploads.push(upload(4, Stage::Failed));
        assert_eq!(uploads.paths(&[1, 3]), None, "one is not stored yet");
        assert_eq!(uploads.paths(&[1]), Some(vec!["/uploads/1/a".to_owned()]));
        // Looked up for a drop that never came: the page never had it.
        assert_eq!(uploads.paths(&[2]), Some(vec!["/uploads/2/b".to_owned()]));
        uploads.give(&[1]);
        let all = std::mem::take(&mut uploads.uploads);
        uploads.forgotten(all);
        // Failed on the display's side, the launcher may still have stored it.
        assert_eq!(uploads.removing, [2, 3, 4], "the page may still read the one it was given");
        assert_eq!(uploads.given, [(1, 0)]);
    }

    fn sized(client: u32, size: u64, stage: Stage) -> Upload {
        Upload { stage, ..Upload::new(client, client, size, None) }
    }

    #[test]
    fn past_the_room_files_go_those_a_page_was_given_first_then_the_holders_oldest() {
        const QUARTER: u64 = UPLOAD_ROOM / 4;
        let mut uploads = Uploads::default();
        // A lease before gave a page the first two; this one's holder has one stored it has not used, a failed one,
        // whose bytes went as it failed, and a newer one a page was given.
        uploads.uploads.push(sized(1, QUARTER, Stage::Stored("/uploads/1/a".into())));
        uploads.uploads.push(sized(2, QUARTER, Stage::Stored("/uploads/2/b".into())));
        uploads.give(&[1, 2]);
        let before = std::mem::take(&mut uploads.uploads);
        uploads.forgotten(before);
        assert!(uploads.removing.is_empty(), "a page may still read them, and there is room");
        uploads.uploads.push(sized(3, QUARTER, Stage::Stored("/uploads/3/c".into())));
        uploads.uploads.push(sized(4, MAX_UPLOAD, Stage::Failed));
        uploads.uploads.push(sized(5, QUARTER, Stage::Stored("/uploads/5/e".into())));
        uploads.give(&[5]);
        assert_eq!(uploads.bytes(), UPLOAD_ROOM);
        assert!(uploads.room(QUARTER, &[]));
        assert_eq!(uploads.removing, [1], "the oldest file of all went, and no more than the room needed");
        assert!(uploads.gone.is_empty(), "which was no longer the holder's to choose");
        uploads.uploads.push(sized(6, QUARTER, Stage::Receiving));
        assert!(uploads.room(2 * QUARTER, &[]));
        assert_eq!(uploads.removing, [1, 2, 5], "then the holder's a page was given, before an older one unused");
        assert_eq!(uploads.gone, [5], "which the holder hears");
        let clients: Vec<u32> = uploads.uploads.iter().map(|u| u.client).collect();
        assert_eq!(clients, [3, 6], "the failed one went too, holding nothing");
        assert!(uploads.given.is_empty());
    }

    #[test]
    fn an_upload_what_stays_leaves_no_room_for_is_refused_and_forgets_nothing() {
        const HALF: u64 = UPLOAD_ROOM / 2;
        let mut uploads = Uploads::default();
        uploads.uploads.push(sized(1, HALF, Stage::Stored("/uploads/1/a".into())));
        uploads.uploads.push(sized(2, HALF, Stage::Receiving));
        uploads.uploads.push(sized(3, 1, Stage::Stored("/uploads/3/c".into())));
        uploads.chooser = Some(Chooser { launcher: 1, request: 1, chosen: Some(vec![1]) });
        assert!(!uploads.room(1, &[]), "one still arrives and the chooser's choice stays");
        uploads.chooser = None;
        assert!(!uploads.room(1, &[1]), "and so does a drop waiting behind typed text");
        assert_eq!(uploads.uploads.len(), 3);
        assert!(uploads.removing.is_empty() && uploads.gone.is_empty());
        assert!(uploads.room(HALF, &[]));
        assert_eq!(uploads.removing, [1, 3], "the oldest, until the new one fits");
        assert_eq!(uploads.gone, [1, 3]);
        assert!(!uploads.room(UPLOAD_ROOM + 1, &[]), "more than all the room");
    }

    #[test]
    fn the_forgotten_uploads_a_page_was_given_count_within_the_bound() {
        // As many as the bound lets in, every one given to a page, then forgotten as the lease ends: all are kept, and
        // the next upload takes the oldest one's place.
        let mut uploads = Uploads::default();
        for c in 1..=MAX_UPLOADS as u32 {
            assert!(uploads.room(1, &[]));
            uploads.uploads.push(Upload { given: true, ..sized(c, 1, Stage::Stored(format!("/uploads/{c}/f"))) });
        }
        let all = std::mem::take(&mut uploads.uploads);
        uploads.forgotten(all);
        assert_eq!((uploads.given.len(), uploads.removing.len()), (MAX_UPLOADS, 0));
        assert!(uploads.room(1, &[]));
        assert_eq!(uploads.removing, [1], "a new upload takes the oldest one's place");
    }
}
