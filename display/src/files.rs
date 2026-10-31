//! Files between the person and the browser, over `easi_files_v1` (protocol/wayland/easi-files-v1.xml).
//!
//! Only the browser container's launcher binds the global, once per display, before it starts Chromium; the agent's
//! socket never offers it. Files move as descriptors, so the display never opens a path the browser can write. A
//! download streams in on a Unix stream socket the launcher writes as Chromium writes the file; the display only ever
//! reads it with `MSG_DONTWAIT` (a flag of the call, not of the shared description, so the launcher cannot make the
//! compositor block), holds at most one piece of it, and reads only as far as its receiver granted credit: the
//! person's own browser, through the admin application's download endpoint, at their own pace. Downloads go to the
//! person holding control, at that lease epoch; one nobody started receiving is offered to the next holder, one being
//! received ends with the lease.

use std::collections::HashMap;
use std::os::fd::{AsFd, OwnedFd};
use std::time::{Duration, Instant};

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{Interest, Mode, PostAction};

use display_protocol::frame::PAGE_OPEN_WAIT;
use display_protocol::{FILE_CHUNK, FILE_WINDOW, MAX_FILE_NAME, Message};
use smithay::wayland::{Dispatch2, GlobalDispatch2};
use wayland_server::{Client, DataInit, DisplayHandle, New, Resource};

use crate::state::{ClientKind, Data, State, client_kind};
use crate::stream::Viewers;

pub mod protocol {
    #![allow(missing_docs, clippy::all, unused_imports, non_upper_case_globals, non_camel_case_types)]
    use wayland_server;
    use wayland_server::protocol::*;
    pub mod __interfaces {
        use wayland_server::backend as wayland_backend;
        wayland_scanner::generate_interfaces!("../protocol/wayland/easi-files-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_server_code!("../protocol/wayland/easi-files-v1.xml");
}

use protocol::easi_files_v1::{self, EasiFilesV1};

/// Downloads known at once; past it the oldest one nobody receives goes.
const MAX_STREAMS: usize = 16;
/// How often a download being received is read again while it waits on what no event announces (room in its viewer's
/// queue, the launcher's end_download). One out of credit waits for credit, one whose source has nothing yet for its
/// socket.
const PUMP: Duration = Duration::from_millis(2);
/// A receiver that takes nothing for this long (no credit granted, no piece accepted) while its download waits on it
/// has abandoned it. A download whose source is slow waits on nobody, however long it takes.
const NO_CREDIT_FOR: Duration = Duration::from_secs(120);
/// How long after the socket closed the launcher's end_download may take.
const END_WITHIN: Duration = Duration::from_secs(10);

pub struct FilesGlobal;

/// A download: the socket it comes from, and how far the person received it.
struct Stream {
    /// The launcher's id for it (none for a recording, which the display itself writes), and the person's.
    id: Option<u32>,
    file: u32,
    name: String,
    /// The size it was offered with, which it must have.
    offered: Option<u64>,
    socket: crate::relay::Held,
    /// Bytes read from the socket and not sent yet: less than one piece.
    piece: Vec<u8>,
    /// Bytes sent, and the most the receiver allows ([`FILE_WINDOW`] past what it consumed).
    sent: u64,
    credit: u64,
    /// The connection and lease epoch receiving it, once one started.
    reader: Option<(u64, u32)>,
    /// Since when it has waited on its receiver: for credit, or for the viewer's queue to take a piece.
    waiting: Option<Instant>,
    /// When the socket closed, and the size the launcher announced.
    closed: Option<Instant>,
    size: Option<u64>,
    /// Its socket watched for data, while its source has none yet and the receiver has credit for more.
    watch: Option<Watch>,
}

impl Stream {
    fn new(id: Option<u32>, file: u32, name: String, offered: Option<u64>, socket: OwnedFd) -> Stream {
        Stream {
            id,
            file,
            name,
            offered,
            socket: crate::relay::Held::new(socket),
            piece: Vec::with_capacity(FILE_CHUNK),
            sent: 0,
            credit: 0,
            reader: None,
            waiting: None,
            closed: None,
            size: None,
            watch: None,
        }
    }
}

/// A readiness source on a descriptor (a download's socket, an upload's pipe), removed when this goes.
pub struct Watch {
    handle: calloop::LoopHandle<'static, Data>,
    token: calloop::RegistrationToken,
}

impl Watch {
    pub fn new(handle: &calloop::LoopHandle<'static, Data>, token: calloop::RegistrationToken) -> Watch {
        Watch { handle: handle.clone(), token }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.handle.remove(self.token);
    }
}

#[derive(Default)]
pub struct Files {
    /// The launcher bound the global; nobody else may.
    bound: bool,
    /// The launcher's object, while it is connected.
    pub launcher: Option<EasiFilesV1>,
    streams: Vec<Stream>,
    next_file: u32,
    /// Pages the launcher was asked to open and has not answered for, by the display's own number for them: the
    /// connection and the admin's request number to answer, and when they were asked.
    opening: HashMap<u32, Opening>,
    next_open: u32,
    /// The timer reading downloads, while any is being received, and whether it only waits out a receiver's
    /// [`NO_CREDIT_FOR`] (every download being received is out of credit, so only credit can move one).
    pump: Option<(calloop::RegistrationToken, bool)>,
}

impl GlobalDispatch2<EasiFilesV1, State> for FilesGlobal {
    fn bind(
        &self,
        state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<EasiFilesV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let files = data_init.init(resource, FilesGlobal);
        if std::mem::replace(&mut state.files.bound, true) {
            files.post_error(0u32, "easi_files_v1 is bound once, by the browser's launcher");
            return;
        }
        state.files.launcher = Some(files);
    }

    fn can_view(&self, client: &Client) -> bool {
        client_kind(client) == Some(ClientKind::Browser)
    }
}

impl Dispatch2<EasiFilesV1, State> for FilesGlobal {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        files: &EasiFilesV1,
        request: easi_files_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            easi_files_v1::Request::StartDownload { id, data, name, known, size_hi, size_lo } => {
                match stream_socket(&data) {
                    Ok(()) if state.files.streams.iter().any(|s| s.id == Some(id)) => {
                        files.post_error(0u32, "a download id used twice")
                    }
                    Ok(()) => {
                        let size = (known != 0).then_some(u64::from(size_hi) << 32 | u64::from(size_lo));
                        if keep(state, Some(id), data, sanitized(&name), size).is_none() {
                            files.stop_download(id);
                        }
                    }
                    Err(why) => files.post_error(0u32, format!("a refused download: {why}")),
                }
            }
            easi_files_v1::Request::EndDownload { id, size_hi, size_lo } => {
                if let Some(stream) = state.files.streams.iter_mut().find(|s| s.id == Some(id)) {
                    stream.size = Some(u64::from(size_hi) << 32 | u64::from(size_lo));
                }
            }
            easi_files_v1::Request::FailDownload { id } => {
                if let Some(index) = state.files.streams.iter().position(|s| s.id == Some(id)) {
                    gone(state, index, false);
                }
            }
            easi_files_v1::Request::OpenChooser { chooser, multiple } => {
                crate::uploads::open_chooser(state, chooser, multiple != 0)
            }
            easi_files_v1::Request::CloseChooser { chooser } => crate::uploads::close_chooser(state, chooser),
            easi_files_v1::Request::Stored { upload, path } => crate::uploads::stored(state, upload, path),
            easi_files_v1::Request::Failed { upload } => crate::uploads::failed(state, upload),
            easi_files_v1::Request::PageOpened { request, opened } => {
                if let Some(o) = state.files.opening.remove(&request) {
                    answer_open(state, o.conn, o.request, opened != 0);
                }
            }
            easi_files_v1::Request::Destroy => {}
        }
    }

    fn destroyed(&self, state: &mut State, _client: wayland_server::backend::ClientId, files: &EasiFilesV1) {
        if state.files.launcher.as_ref() == Some(files) {
            state.files.launcher = None;
            for (_, o) in std::mem::take(&mut state.files.opening) {
                answer_open(state, o.conn, o.request, false);
            }
            while !state.files.streams.is_empty() {
                gone(state, 0, false);
            }
            crate::uploads::lease_changed(state);
        }
    }
}

/// A download's descriptor must be a Unix stream socket: read only with `MSG_DONTWAIT`, it can never block the
/// compositor, whatever flags the launcher sets on its side.
fn stream_socket(fd: &OwnedFd) -> Result<(), &'static str> {
    use rustix::fs::FileType;
    let stat = rustix::fs::fstat(fd.as_fd()).map_err(|_| "fstat")?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Socket {
        return Err("not a socket");
    }
    if rustix::net::sockopt::socket_type(fd.as_fd()).map_err(|_| "socket type")? != rustix::net::SocketType::STREAM {
        return Err("not a stream socket");
    }
    let address = rustix::net::getsockname(fd.as_fd()).map_err(|_| "socket address")?;
    if address.address_family() != rustix::net::AddressFamily::UNIX {
        return Err("not a Unix socket");
    }
    Ok(())
}

/// A name the person's browser can save under: no path separators or control characters, at most [`MAX_FILE_NAME`]
/// bytes, never empty or a dot name.
pub fn sanitized(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        let c = if c.is_control() || matches!(c, '/' | '\\') { '_' } else { c };
        if out.len() + c.len_utf8() > MAX_FILE_NAME {
            break;
        }
        out.push(c);
    }
    let out = out.trim().trim_start_matches('.').to_owned();
    if out.is_empty() { "download".to_owned() } else { out }
}

/// Keeps a new download, making room by dropping the oldest one nobody receives, and offers it to the holder; its
/// file number, or none when there is no room.
fn keep(state: &mut State, id: Option<u32>, socket: OwnedFd, name: String, size: Option<u64>) -> Option<u32> {
    if state.files.streams.len() >= MAX_STREAMS {
        match state.files.streams.iter().position(|s| s.reader.is_none()) {
            Some(oldest) => gone(state, oldest, true),
            None => {
                tracing::info!("no room for a download while others are being received; refused");
                return None;
            }
        }
    }
    let files = &mut state.files;
    files.next_file = files.next_file.wrapping_add(1).max(1);
    let file = files.next_file;
    files.streams.push(Stream::new(id, file, name.clone(), size, socket));
    // An offer that does not go now goes to the next holder (see [`lease_changed`]).
    if let Some((conn, lease_epoch)) = state.lease.current() {
        let offer = Message::FileOffer { lease_epoch, file, size, name };
        state.hub.viewers.send_notice_to(conn, &offer);
    }
    Some(file)
}

/// A recording's download: the display writes it into `socket` as it is made (its size is known only at the end, given
/// by [`end_recording`]). Its file number, or none when there is no room.
pub fn start_recording(state: &mut State, socket: OwnedFd, name: String) -> Option<u32> {
    keep(state, None, socket, name, None)
}

/// The recording `file` was written whole: `size` bytes.
pub fn end_recording(state: &mut State, file: u32, size: u64) {
    if let Some(stream) = state.files.streams.iter_mut().find(|s| s.file == file && s.id.is_none()) {
        stream.size = Some(size);
        pump_soon(state);
    }
}

/// The recording `file` failed: its download goes.
pub fn fail_recording(state: &mut State, file: u32) {
    if let Some(index) = state.files.streams.iter().position(|s| s.file == file && s.id.is_none()) {
        gone(state, index, false);
    }
}

/// Ends the download at `index`: the person is told it is gone, and the launcher (unless it said so itself) that the
/// display reads no more of it.
fn gone(state: &mut State, index: usize, tell_launcher: bool) {
    let stream = state.files.streams.remove(index);
    if tell_launcher && let (Some(launcher), Some(id)) = (&state.files.launcher, stream.id) {
        launcher.stop_download(id);
    }
    tell_gone(&state.hub.viewers, stream.reader, state.lease.current(), stream.file);
    // A recording whose download went (cancelled, unread, the lease moved) cannot go on.
    if stream.id.is_none() {
        crate::recording::download_gone(state, stream.file);
    }
}

/// Tells that `file` is gone: to its receiver, as the end of a transfer it started (its first read), never dropped nor
/// a reason to disconnect it, since one that consumed all it was sent asks for nothing more and a dropped end would
/// leave it waiting. Otherwise to `holder`, whom it was offered to, as a notice (the launcher can end downloads as fast
/// as it starts them): one that missed it hears it again when it asks for the download.
fn tell_gone(viewers: &Viewers, reader: Option<(u64, u32)>, holder: Option<(u64, u32)>, file: u32) {
    if let Some((conn, lease_epoch)) = reader {
        viewers.send_end_to(conn, &Message::FileGone { lease_epoch, file });
    } else if let Some((conn, lease_epoch)) = holder {
        viewers.send_notice_to(conn, &Message::FileGone { lease_epoch, file });
    }
}

/// The lease moved: recordings end, whoever was reading them (they hold the previous holder's screen, camera and
/// microphone); the browser's downloads being received by the previous holder end; the others are offered to the new
/// holder.
pub fn lease_changed(state: &mut State) {
    // Pages asked for by the previous holder: not theirs to open any more (the launcher's late answers find nothing).
    for (_, o) in std::mem::take(&mut state.files.opening) {
        answer_open(state, o.conn, o.request, false);
    }
    while let Some(index) = state.files.streams.iter().position(|s| s.id.is_none()) {
        let stream = state.files.streams.remove(index);
        tell_gone(&state.hub.viewers, stream.reader, None, stream.file);
        crate::recording::download_gone(state, stream.file);
    }
    let current = state.lease.current();
    while let Some(index) = state.files.streams.iter().position(|s| s.reader.is_some_and(|r| Some(r) != current)) {
        gone(state, index, true);
    }
    offer_waiting(state);
}

/// Offers every download nobody receives to whoever files go to now (see `Lease::current`).
pub fn offer_waiting(state: &State) {
    if let Some((conn, lease_epoch)) = state.lease.current() {
        for stream in state.files.streams.iter().filter(|s| s.reader.is_none()) {
            let offer =
                Message::FileOffer { lease_epoch, file: stream.file, size: stream.offered, name: stream.name.clone() };
            state.hub.viewers.send_notice_to(conn, &offer);
        }
    }
}

/// Pages waiting for the launcher at once; one waiting past [`PAGE_OPEN_WAIT`] is answered as not opened.
const MAX_OPENING: usize = 4;

pub struct Opening {
    conn: u64,
    request: u32,
    at: Instant,
}

/// The admin asks, on the lease holder's connection `conn` at `lease_epoch`, that the browser open `url` in place of
/// its pages (a demo switch). Answered on that connection with `PageOpened`, now if it cannot go.
pub fn open_page(state: &mut State, conn: u64, lease_epoch: u32, request: u32, url: String) {
    let expired: Vec<u32> =
        state.files.opening.iter().filter(|(_, o)| o.at.elapsed() >= PAGE_OPEN_WAIT).map(|(id, _)| *id).collect();
    for id in expired {
        if let Some(o) = state.files.opening.remove(&id) {
            answer_open(state, o.conn, o.request, false);
        }
    }
    let holder = state.lease.holds(conn, lease_epoch);
    let room = state.files.opening.len() < MAX_OPENING;
    match &state.files.launcher {
        Some(launcher) if holder && room => {
            // The launcher hears the display's own number: requests of different connections never mix up.
            state.files.next_open = state.files.next_open.wrapping_add(1);
            let id = state.files.next_open;
            launcher.open_page(id, url);
            state.files.opening.insert(id, Opening { conn, request, at: Instant::now() });
        }
        _ => answer_open(state, conn, request, false),
    }
}

fn answer_open(state: &State, conn: u64, request: u32, opened: bool) {
    let link = display_protocol::frame::Link::PageOpened { request, opened };
    state.hub.viewers.send_link_to(conn, &link);
}

/// The holder consumed `file` up to `offset` (the lease was already checked): more may be sent.
pub fn read(state: &mut State, conn: u64, file: u32, offset: u64) {
    let epoch = state.lease.epoch;
    let Some(index) = state.files.streams.iter().position(|s| s.file == file) else {
        state.hub.viewers.send_control_to(conn, &Message::FileGone { lease_epoch: epoch, file });
        return;
    };
    let stream = &mut state.files.streams[index];
    let first = stream.reader.is_none();
    // Credit only grows, and never past what was sent: the receiver cannot have consumed more.
    let valid = match stream.reader {
        None => offset == 0,
        Some(reader) => reader == (conn, epoch) && offset <= stream.sent && offset + FILE_WINDOW >= stream.credit,
    };
    if !valid {
        gone(state, index, true);
        return;
    }
    stream.reader = Some((conn, epoch));
    stream.credit = offset + FILE_WINDOW;
    stream.waiting = None;
    if first && let (Some(launcher), Some(id)) = (&state.files.launcher, stream.id) {
        launcher.reading(id);
    }
    pump_soon(state);
}

/// The holder no longer wants `file`.
pub fn cancel(state: &mut State, conn: u64, file: u32) {
    let epoch = state.lease.epoch;
    if let Some(index) =
        state.files.streams.iter().position(|s| s.file == file && s.reader.is_none_or(|r| r == (conn, epoch)))
    {
        gone(state, index, true);
    }
}

/// Starts the pump timer, unless it is already reading: one that only waits out a deadline gives way to reading now.
fn pump_soon(state: &mut State) {
    match state.files.pump {
        Some((_, false)) => return,
        Some((token, true)) => state.handle.remove(token),
        None => {}
    }
    let timer = state.handle.insert_source(Timer::from_duration(PUMP), |_, _, data: &mut Data| {
        let state = &mut data.state;
        // Reading now: credit granted meanwhile finds the timer running.
        if let Some((_, waiting)) = &mut state.files.pump {
            *waiting = false;
        }
        match pump(state) {
            Some(after) => {
                if let Some((_, waiting)) = &mut state.files.pump {
                    *waiting = after > PUMP;
                }
                TimeoutAction::ToDuration(after)
            }
            None => {
                state.files.pump = None;
                TimeoutAction::Drop
            }
        }
    });
    state.files.pump = timer.ok().map(|token| (token, false));
}

/// What one download needs after it was read.
enum Next {
    /// Nobody receives it yet.
    Unread,
    /// To be read again soon.
    Wait,
    /// Nothing until its source writes more.
    Readable,
    /// Nothing until its receiver grants credit, at most this long.
    Credit(Duration),
    End,
    Fail,
}

/// Sends what every download being received has and may send; how soon to come back while any is still being
/// received.
fn pump(state: &mut State) -> Option<Duration> {
    let mut after: Option<Duration> = None;
    let mut index = 0;
    while index < state.files.streams.len() {
        let wait = match pump_one(state, index) {
            Next::Unread => {
                index += 1;
                continue;
            }
            Next::Readable if watch(state, index) => {
                index += 1;
                continue;
            }
            Next::Wait | Next::Readable => PUMP,
            Next::Credit(left) => left.max(PUMP),
            Next::End => {
                state.files.streams.remove(index);
                continue;
            }
            Next::Fail => {
                gone(state, index, true);
                continue;
            }
        };
        after = Some(after.map_or(wait, |after| after.min(wait)));
        index += 1;
    }
    after
}

fn pump_one(state: &mut State, index: usize) -> Next {
    let State { files, hub, .. } = state;
    let stream = &mut files.streams[index];
    let Some((conn, lease_epoch)) = stream.reader else { return Next::Unread };
    loop {
        if stream.offered.is_some_and(|size| stream.sent + stream.piece.len() as u64 > size) {
            return Next::Fail;
        }
        // A piece ready goes when the credit and the viewer's bulk budget allow; the last one may be short.
        let full = stream.piece.len() == FILE_CHUNK;
        let ready = full || (stream.closed.is_some() && !stream.piece.is_empty());
        if ready && stream.sent + stream.piece.len() as u64 <= stream.credit {
            // The piece moves into the message, and back when the viewer's queue has no room for it.
            let data = std::mem::take(&mut stream.piece);
            let length = data.len() as u64;
            let message = Message::FileData { lease_epoch, file: stream.file, offset: stream.sent, data };
            if !hub.viewers.send_bulk_to(conn, &message) {
                if let Message::FileData { data, .. } = message {
                    stream.piece = data;
                }
                return wait_on_receiver(stream).map_or(Next::Fail, |_| Next::Wait);
            }
            stream.sent += length;
            stream.waiting = None;
            continue;
        }
        // Out of credit: a piece waits that may not go, or no room is left for one to be read.
        if ready || (stream.closed.is_none() && stream.sent + FILE_CHUNK as u64 > stream.credit) {
            return wait_on_receiver(stream).map_or(Next::Fail, Next::Credit);
        }
        if stream.closed.is_some() {
            break;
        }
        let at = stream.piece.len();
        stream.piece.resize(FILE_CHUNK, 0);
        match rustix::net::recv(&stream.socket, &mut stream.piece[at..], rustix::net::RecvFlags::DONTWAIT) {
            Ok((0, _)) => {
                stream.piece.truncate(at);
                stream.closed = Some(Instant::now());
            }
            Ok((n, _)) => stream.piece.truncate(at + n),
            Err(rustix::io::Errno::AGAIN) => {
                stream.piece.truncate(at);
                return Next::Readable;
            }
            Err(rustix::io::Errno::INTR) => {
                stream.piece.truncate(at);
                break;
            }
            Err(_) => return Next::Fail,
        }
    }
    let Some(closed) = stream.closed else { return Next::Wait };
    if !stream.piece.is_empty() {
        return Next::Wait;
    }
    match stream.size {
        // FileEnd follows the last piece through the same queue, so it cannot overtake it.
        Some(size) if size == stream.sent && stream.offered.is_none_or(|o| o == size) => {
            let end = Message::FileEnd { lease_epoch, file: stream.file, size };
            if hub.viewers.send_bulk_to(conn, &end) {
                Next::End
            } else {
                wait_on_receiver(stream).map_or(Next::Fail, |_| Next::Wait)
            }
        }
        Some(_) => Next::Fail,
        None if closed.elapsed() >= END_WITHIN => Next::Fail,
        None => Next::Wait,
    }
}

/// Watches the socket of the download at `index` for data, which pumps the downloads again: a slow source is not read
/// every [`PUMP`] for nothing while it writes nothing. False when it cannot be watched, so it is read again soon.
fn watch(state: &mut State, index: usize) -> bool {
    let stream = &state.files.streams[index];
    if stream.watch.is_some() {
        return true;
    }
    let file = stream.file;
    let Ok(fd) = rustix::io::fcntl_dupfd_cloexec(&stream.socket, 0) else { return false };
    let source = Generic::new(crate::relay::Held::new(fd), Interest::READ, Mode::Level);
    let token = state.handle.insert_source(source, move |_, _, data: &mut Data| {
        if let Some(stream) = data.state.files.streams.iter_mut().find(|s| s.file == file) {
            stream.watch = None;
        }
        pump_soon(&mut data.state);
        Ok(PostAction::Remove)
    });
    let Ok(token) = token else { return false };
    state.files.streams[index].watch = Some(Watch::new(&state.handle, token));
    true
}

/// The download waits on its receiver: how much longer it may, none once the receiver abandoned it.
fn wait_on_receiver(stream: &mut Stream) -> Option<Duration> {
    let since = *stream.waiting.get_or_insert_with(Instant::now);
    NO_CREDIT_FOR.checked_sub(since.elapsed()).filter(|left| !left.is_zero())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_waits_on_its_receiver_only_so_long() {
        let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let mut stream = Stream { reader: Some((1, 1)), ..Stream::new(Some(1), 1, "a".into(), None, socket.into()) };
        assert!(wait_on_receiver(&mut stream).is_some_and(|left| left <= NO_CREDIT_FOR));
        assert!(stream.waiting.is_some(), "the clock starts once it waits on the receiver");
        stream.waiting = Instant::now().checked_sub(NO_CREDIT_FOR);
        assert_eq!(wait_on_receiver(&mut stream), None);
    }

    /// A receiver that consumed all it was sent (here, nothing) asks for nothing more: however many notices fill its
    /// queue, it hears that its download is gone. One only offered the download misses it and hears it again once it
    /// asks.
    #[test]
    fn a_download_being_received_is_told_it_is_gone_past_the_notices() {
        let viewers = Viewers::default();
        let mut queued = viewers.never_reading(7);
        let offer = Message::FileOffer { lease_epoch: 1, file: 9, size: None, name: "a".into() };
        while viewers.send_notice_to(7, &offer) {}
        queued();
        tell_gone(&viewers, Some((7, 1)), Some((7, 1)), 1);
        tell_gone(&viewers, None, Some((7, 1)), 2);
        assert_eq!(queued(), [Message::FileGone { lease_epoch: 1, file: 1 }]);
    }

    #[test]
    fn names_lose_paths_and_control_characters_and_keep_their_bound() {
        assert_eq!(sanitized("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitized("rel\u{0}at\nório.pdf"), "rel_at_ório.pdf");
        assert_eq!(sanitized(".hidden"), "hidden");
        assert_eq!(sanitized("  "), "download");
        let long = "é".repeat(300);
        let cut = sanitized(&long);
        assert!(cut.len() <= MAX_FILE_NAME && cut.chars().all(|c| c == 'é'));
    }
}
