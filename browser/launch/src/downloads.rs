//! Downloads streamed from Chromium's own files to the display while Chromium writes them.
//!
//! Every file created in /downloads is followed by inode through its renames (inotify: create, both halves of a move,
//! open, close-after-write). Chromium writes a new download into a temporary dot file, renames it to
//! `<name>.crdownload` once its target is known, and renames that to `<name>` when it is complete. From that first
//! move on the download is streamed: read in order up to its size, written to a socket pair whose other end the
//! display holds, and the pages handed on are punched out of the file, so a download of any size stays within a few
//! tens of MiB of the tmpfs, at the person's pace (see `gate`).
//!
//! The person must get exactly the bytes of Chromium's final file, or a failed download. Chromium truncates or
//! rewrites a download only when a new writer opens it (a resumption or restart after an interruption), and the
//! previous writer closed it first. So:
//! - the fence: once streaming, a close-after-write freezes the stream; the next close-or-move of the file must be
//!   its move to a plain name (Chromium's Rename closes, then renames), else it fails. Two generations' closes always
//!   have the second open between them, so they never merge in the queue; a queue overflow fails every stream;
//! - happens-before: after every read, and before its bytes go on, the queued events are handled, so a read that saw
//!   any byte of a later generation finds the earlier generation's close and drops them;
//! - the first move itself may expose a file whose writer already closed (interrupted before its target was known),
//!   which no event distinguishes, so the bytes written by then are pinned (never punched) and their hash compared
//!   with the file once it is complete: a different prefix fails the download.
//!
//! Everything else is delivered only complete (an "Unconfirmed" intermediate, a download whose prefix is too large to
//! pin, a temporary file never moved, a page's save renamed into place), and nothing is ever taken that is written in
//! place without a rename, but one: a page saved as one file (MHTML), which Chromium writes in place into the name the
//! portal reserved for it, and closes once, after its last part. It is delivered at that close only if it then ends with
//! the closing boundary its own header declares, which Chromium writes last: a save cut short never has it.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{ErrorKind, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::{FileExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustix::fs::inotify::{self, ReadFlags};
use rustix::fs::{AtFlags, FallocateFlags, FileType, Mode, OFlags};
use sha2::{Digest, Sha256};

use crate::cdp::{Lookup, Sizes};
use crate::files::protocol::easi_files_v1::EasiFilesV1;
use crate::gate::{Gate, Held};
use crate::portal::Shared;

/// Chromium's suffix for a download in progress, and its prefix for one whose target is not confirmed.
const PARTIAL: &str = ".crdownload";
const UNCONFIRMED: &str = "Unconfirmed ";
const MIB: u64 = 1024 * 1024;
/// Largest prefix one stream may pin, and all of them together.
const PIN: u64 = 16 * MIB;
const PINNED_TOTAL: u64 = 48 * MIB;
/// Bytes read from a file at once.
const STEP: usize = 256 * 1024;
/// A stream whose file was closed must reach its plain name within this time.
const CLOSE_WITHIN: Duration = Duration::from_secs(5);
/// A temporary file never moved is delivered only complete after this time.
const TEMPORARY_FOR: Duration = Duration::from_secs(5);
/// Files followed at once.
const MAX_TRACKED: usize = 256;
/// How long a download's announcement waits for Chromium to say its size.
const SIZE_WITHIN: Duration = Duration::from_secs(1);
/// How long an announced download nobody receives stays under the gate's allowance (an agent's, with no person).
const UNRECEIVED_FOR: Duration = Duration::from_secs(5);
/// How often files are looked at while any is being streamed or discarded.
pub const TICK: Duration = Duration::from_millis(10);

pub struct Downloads {
    notify: OwnedFd,
    dir: OwnedFd,
    /// The launcher's own directory inside it, not watched, where finished files are removed.
    aside: OwnedFd,
    files: EasiFilesV1,
    gate: Arc<Gate>,
    shared: Arc<Shared>,
    sizes: Arc<Sizes>,
    tracked: Vec<Tracked>,
    /// The first half of a move: its cookie, the file it moved if followed, and whether its old name was temporary.
    moving: Option<(u32, Option<u64>, bool)>,
    next_id: u32,
}

struct Tracked {
    ino: u64,
    name: CString,
    file: Arc<File>,
    phase: Phase,
    /// When it was first followed: a candidate stays under the gate's allowance at most [`TEMPORARY_FOR`].
    since: Instant,
}

enum Phase {
    /// Chromium's temporary file: not read until it is named.
    Temporary(Instant),
    /// Delivered only once complete.
    Waiting,
    /// Any other file (a reserved name, a page's save in place): delivered only if renamed to a plain name, or, a page
    /// saved as one file into its reservation, once whole (see [`whole_mhtml`]).
    Other,
    Streaming(Box<Stream>),
    /// Its bytes are gone (handed on, then refused or failed): punched as Chromium writes, removed when complete.
    Discarding,
    /// Forgotten at the end of this pass.
    Gone,
}

struct Stream {
    id: u32,
    socket: UnixStream,
    /// The display's end and name until it is announced, which waits up to [`SIZE_WITHIN`] for Chromium to say its
    /// size; when it was announced; and whether it left the gate's allowance (received, or unreceived for
    /// [`UNRECEIVED_FOR`]).
    theirs: Option<UnixStream>,
    shown: String,
    born: Instant,
    announced: Option<Instant>,
    free: bool,
    /// Bytes the socket took, and the last read with how much of it the socket took (emptied once it took it all).
    sent: u64,
    pending: Vec<u8>,
    pending_at: usize,
    punched: u64,
    /// The prefix written before the first move, and the hash of what was handed on of it.
    pinned: u64,
    hasher: Sha256,
    received: bool,
    /// When the file was closed (the fence), and its size once it reached its plain name (no writer then).
    closing: Option<Instant>,
    done: Option<u64>,
}

/// What a name says about the file it names.
#[derive(PartialEq)]
enum Kind {
    Temporary,
    Partial,
    Unconfirmed,
    Plain,
}

/// Header bytes searched for an MHTML file's boundary: room for a page's title of any length Chromium copies into it.
const MHTML_HEADER: u64 = 1024 * 1024;

/// Whether `file` is a whole MHTML file: its header's own `Content-Type` field (the page's title, which Chromium copies
/// into `Subject`, may hold anything printable, but no line break) names a multipart boundary, and that boundary's
/// closing delimiter (`--boundary--`) is its last line.
fn whole_mhtml(file: &File) -> bool {
    let Ok(size) = file.metadata().map(|m| m.len()) else { return false };
    let mut head = vec![0; size.min(MHTML_HEADER) as usize];
    if file.read_exact_at(&mut head, 0).is_err() {
        return false;
    }
    let Some(end) = head.windows(4).position(|w| w == b"\r\n\r\n") else { return false };
    // The header's fields, unfolded: a line that starts with a space or a tab goes on with the field before it.
    let mut fields: Vec<String> = Vec::new();
    for line in String::from_utf8_lossy(&head[..end]).split("\r\n") {
        match fields.last_mut() {
            Some(field) if line.starts_with([' ', '\t']) => field.push_str(line),
            _ => fields.push(line.to_owned()),
        }
    }
    let content_type = fields.iter().find_map(|field| {
        let (name, value) = field.split_once(':')?;
        name.trim().eq_ignore_ascii_case("content-type").then_some(value)
    });
    let boundary = content_type.and_then(|value| {
        value.split(';').find_map(|parameter| {
            let (name, value) = parameter.split_once('=')?;
            name.trim().eq_ignore_ascii_case("boundary").then(|| value.trim().trim_matches('"'))
        })
    });
    let Some(boundary) = boundary.filter(|b| !b.is_empty()) else { return false };
    let closing = format!("--{boundary}--");
    let length = (closing.len() as u64 + 8).min(size);
    let mut tail = vec![0; length as usize];
    if file.read_exact_at(&mut tail, size - length).is_err() {
        return false;
    }
    let tail = tail.trim_ascii_end();
    tail.ends_with(closing.as_bytes()) && tail.len() > closing.len() && tail[tail.len() - closing.len() - 1] == b'\n'
}

fn kind(name: &CStr) -> Kind {
    let text = name.to_string_lossy();
    if text.starts_with('.') {
        Kind::Temporary
    } else if text.ends_with(PARTIAL) && text.starts_with(UNCONFIRMED) {
        Kind::Unconfirmed
    } else if text.ends_with(PARTIAL) {
        Kind::Partial
    } else {
        Kind::Plain
    }
}

impl Downloads {
    pub fn new(
        (notify, dir, aside): (OwnedFd, OwnedFd, OwnedFd),
        files: EasiFilesV1,
        gate: Arc<Gate>,
        shared: Arc<Shared>,
        sizes: Arc<Sizes>,
    ) -> Downloads {
        Downloads { notify, dir, aside, files, gate, shared, sizes, tracked: Vec::new(), moving: None, next_id: 0 }
    }

    pub fn notify(&self) -> &OwnedFd {
        &self.notify
    }

    /// The sockets that have bytes waiting for room.
    pub fn blocked_sockets(&self) -> Vec<&UnixStream> {
        self.tracked
            .iter()
            .filter_map(|t| match &t.phase {
                Phase::Streaming(s) if !s.pending.is_empty() => Some(&s.socket),
                _ => None,
            })
            .collect()
    }

    /// How long the loop may sleep: a tick while anything is streamed, discarded, or temporary.
    pub fn timeout(&self) -> Option<Duration> {
        let busy = self.tracked.iter().any(|t| match t.phase {
            Phase::Streaming(_) | Phase::Discarding | Phase::Temporary(_) => true,
            // Until a reservation's allowance has run out.
            Phase::Other => t.since.elapsed() < TEMPORARY_FOR + 2 * TICK,
            _ => false,
        });
        busy.then_some(TICK)
    }

    /// The display started receiving download `id`.
    pub fn reading(&mut self, id: u32) {
        let Some((ino, stream)) = self.stream_mut(id) else { return };
        stream.received = true;
        stream.free = true;
        self.held(ino, |held| {
            held.received = true;
            held.pre = false;
        });
    }

    /// The display reads no more of download `id`: the rest is discarded (a complete file is removed at once).
    pub fn stop(&mut self, id: u32) {
        if let Some(index) = self.tracked.iter().position(|t| matches!(&t.phase, Phase::Streaming(s) if s.id == id)) {
            self.discard(index);
        }
    }

    fn discard(&mut self, index: usize) {
        let complete = matches!(&self.tracked[index].phase, Phase::Streaming(s) if s.done.is_some());
        if complete {
            self.remove(index);
        } else {
            self.tracked[index].phase = Phase::Discarding;
            self.not_received(self.tracked[index].ino);
        }
    }

    fn stream_mut(&mut self, id: u32) -> Option<(u64, &mut Stream)> {
        self.tracked.iter_mut().find_map(|t| match &mut t.phase {
            Phase::Streaming(s) if s.id == id => Some((t.ino, s.as_mut())),
            _ => None,
        })
    }

    /// Changes what the gate holds of `ino`, if it holds it.
    fn held(&self, ino: u64, change: impl FnOnce(&mut Held)) {
        if let Some(held) = self.gate.held.lock().unwrap().get_mut(&ino) {
            change(held);
        }
    }

    fn not_received(&self, ino: u64) {
        self.held(ino, |held| {
            held.received = false;
            held.pinned = 0;
            held.pre = false;
        });
    }

    /// A temporary file left the gate's allowance: admitted, or delivered only complete.
    fn admitted(&self, ino: u64) {
        self.held(ino, |held| held.pre = false);
    }

    /// Handles every queued event.
    pub fn events(&mut self) -> Result<(), String> {
        let mut buffer = [std::mem::MaybeUninit::<u8>::uninit(); 4096];
        loop {
            let mut reader = inotify::Reader::new(&self.notify, &mut buffer);
            let mut batch = Vec::new();
            loop {
                match reader.next() {
                    Ok(event) => batch.push((event.events(), event.cookie(), event.file_name().map(CStr::to_owned))),
                    Err(rustix::io::Errno::AGAIN) => break,
                    Err(e) => return Err(format!("inotify: {e}")),
                }
            }
            if batch.is_empty() {
                return Ok(());
            }
            for (events, cookie, name) in batch {
                self.event(events, cookie, name);
            }
        }
    }

    fn event(&mut self, events: ReadFlags, cookie: u32, name: Option<CString>) {
        if events.contains(ReadFlags::QUEUE_OVERFLOW) {
            return self.overflowed();
        }
        let Some(name) = name else { return };
        if events.contains(ReadFlags::CREATE) {
            self.created(&name);
        } else if events.contains(ReadFlags::MOVED_FROM) {
            let from = self.by_name(&name).map(|i| self.tracked[i].ino);
            self.moving = Some((cookie, from, kind(&name) == Kind::Temporary));
        } else if events.contains(ReadFlags::MOVED_TO) {
            let (from, temporary) = match self.moving.take() {
                Some((c, from, temporary)) if c == cookie => (from, temporary),
                _ => (None, false),
            };
            self.moved(from, temporary, name);
        } else if events.contains(ReadFlags::CLOSE_WRITE)
            && let Some(i) = self.by_name(&name)
        {
            match &mut self.tracked[i].phase {
                Phase::Streaming(s) if s.done.is_none() => {
                    if s.closing.is_none() {
                        s.closing = Some(Instant::now());
                    } else {
                        // A second writer closed it before it was complete: another generation wrote it.
                        self.fail(i);
                    }
                }
                Phase::Other => self.saved_page(i, &name),
                _ => {}
            }
        }
    }

    /// A page Chromium saved as one file into the name the portal reserved for it: delivered once whole, and the name
    /// forgotten then. Its reservation's own creation closes it empty, which is not whole.
    fn saved_page(&mut self, index: usize, name: &CStr) {
        let name = name.to_string_lossy();
        if !name.ends_with(crate::portal::SINGLE_FILE) || !self.shared.saves.lock().unwrap().contains_key(name.as_ref())
        {
            return;
        }
        if whole_mhtml(&self.tracked[index].file) {
            self.shared.saves.lock().unwrap().remove(name.as_ref());
            self.complete(index);
        }
    }

    /// The index of `ino` if it is followed.
    fn followed(&self, ino: u64) -> Option<usize> {
        self.tracked.iter().position(|t| t.ino == ino && !matches!(t.phase, Phase::Gone))
    }

    fn by_name(&self, name: &CStr) -> Option<usize> {
        self.tracked.iter().position(|t| !matches!(t.phase, Phase::Gone) && t.name.as_c_str() == name)
    }

    /// Events were lost: nothing streamed can be trusted, and names are found again by inode.
    fn overflowed(&mut self) {
        eprintln!("browser-launch: downloads: events were lost; downloads in progress fail");
        for i in 0..self.tracked.len() {
            match self.tracked[i].phase {
                Phase::Streaming(_) => self.fail(i),
                Phase::Temporary(_) => self.tracked[i].phase = Phase::Waiting,
                _ => {}
            }
            self.admitted(self.tracked[i].ino);
        }
        let Ok(entries) = rustix::fs::Dir::read_from(&self.dir) else { return };
        let mut names = HashMap::new();
        for entry in entries.flatten() {
            names.insert(entry.ino(), entry.file_name().to_owned());
        }
        for t in &mut self.tracked {
            t.name = names.get(&t.ino).cloned().unwrap_or_default();
        }
        // Files whose creation was lost: followed, delivered only complete if Chromium completes them.
        for (_, name) in names {
            if self.by_name(&name).is_none()
                && let Some((ino, file)) = self.open(&name, false)
            {
                self.track(ino, name, file, Phase::Waiting);
            }
        }
    }

    /// Opens a file by name, without following a link or blocking, if it is a regular file: for writing (to punch
    /// it) only when it may be streamed, since closing a file opened for writing is an event the fence reads.
    fn open(&self, name: &CStr, writable: bool) -> Option<(u64, Arc<File>)> {
        let access = if writable { OFlags::RDWR } else { OFlags::RDONLY };
        let flags = access | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOCTTY;
        let fd = rustix::fs::openat(&self.dir, name, flags, Mode::empty()).ok()?;
        let stat = rustix::fs::fstat(&fd).ok()?;
        (FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile)
            .then(|| (stat.st_ino, Arc::new(File::from(fd))))
    }

    fn track(&mut self, ino: u64, name: CString, file: Arc<File>, phase: Phase) -> Option<usize> {
        if let Some(i) = self.followed(ino) {
            return Some(i);
        }
        if self.tracked.len() >= MAX_TRACKED {
            return None;
        }
        // A temporary file, or a `.crdownload` name (an empty reservation, or a temporary file renamed over it before
        // its creation was handled), stays under the gate's allowance until a move classifies it.
        let pre =
            matches!(phase, Phase::Temporary(_)) || (matches!(phase, Phase::Other) && kind(&name) == Kind::Partial);
        self.gate.held.lock().unwrap().insert(ino, Held { file: file.clone(), pinned: 0, received: false, pre });
        self.tracked.push(Tracked { ino, name, file, phase, since: Instant::now() });
        Some(self.tracked.len() - 1)
    }

    /// A new name. What it names is opened now, so it may already be another file renamed over it: one followed
    /// already keeps its own history. Chromium creates `<name>.crdownload` and `<name>` empty, to reserve them, and
    /// renames its temporary file over them; only a temporary file is ever read.
    fn created(&mut self, name: &CStr) {
        let Some((ino, file)) = self.open(name, kind(name) == Kind::Temporary) else { return };
        let phase = match kind(name) {
            Kind::Temporary => Phase::Temporary(Instant::now()),
            Kind::Unconfirmed => Phase::Waiting,
            Kind::Partial | Kind::Plain => Phase::Other,
        };
        self.track(ino, name.to_owned(), file, phase);
    }

    fn moved(&mut self, from: Option<u64>, temporary: bool, name: CString) {
        // A file the move replaced is gone.
        if let Some(i) = self.by_name(&name)
            && Some(self.tracked[i].ino) != from
        {
            self.forget(i);
        }
        let index = match from.and_then(|ino| self.followed(ino)) {
            Some(i) => i,
            None => {
                // Not followed: renamed before its creation was handled, or never seen. A temporary file keeps its
                // temporary history; anything else is delivered only complete.
                let temporary = temporary || kind(&name) == Kind::Temporary;
                let Some((ino, file)) = self.open(&name, temporary) else { return };
                let phase = if temporary { Phase::Temporary(Instant::now()) } else { Phase::Waiting };
                let Some(i) = self.track(ino, name.clone(), file, phase) else { return };
                i
            }
        };
        let to = kind(&name);
        self.tracked[index].name = name.clone();
        if to == Kind::Plain {
            self.shared.saves.lock().unwrap().remove(name.to_string_lossy().as_ref());
        }
        let t = &mut self.tracked[index];
        let size = t.file.metadata().map(|m| m.len()).unwrap_or(0);
        match (&mut t.phase, to) {
            (Phase::Temporary(_), Kind::Partial) => self.start(index),
            (Phase::Temporary(_), Kind::Unconfirmed) => {
                t.phase = Phase::Waiting;
                let ino = t.ino;
                self.admitted(ino);
            }
            (Phase::Temporary(_) | Phase::Waiting | Phase::Other, Kind::Plain) => self.complete(index),
            // No writer is open once Chromium renamed it complete (its Rename closes first).
            (Phase::Streaming(s), Kind::Plain) => s.done = Some(size),
            (Phase::Streaming(_), _) => self.fail(index),
            (Phase::Discarding, Kind::Plain) => self.remove(index),
            _ => {}
        }
    }

    /// Starts streaming a file named `<name>.crdownload`, pinning what it holds now; delivered complete instead if that
    /// is more than may be pinned.
    fn start(&mut self, index: usize) {
        let size = self.tracked[index].file.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
        let pinned: u64 = self
            .tracked
            .iter()
            .map(|t| match &t.phase {
                Phase::Streaming(s) => s.pinned,
                _ => 0,
            })
            .sum();
        if size > PIN || pinned + size > PINNED_TOTAL {
            self.tracked[index].phase = Phase::Waiting;
            self.admitted(self.tracked[index].ino);
            return;
        }
        let shown = self.tracked[index].name.to_string_lossy();
        let shown = shown.strip_suffix(PARTIAL).unwrap_or(&shown).to_owned();
        self.stream(index, shown, size, None);
    }

    /// Streams a complete file (it reached its plain name; nothing writes it any more).
    fn complete(&mut self, index: usize) {
        let size = self.tracked[index].file.metadata().map(|m| m.len()).unwrap_or(0);
        let shown = self.tracked[index].name.to_string_lossy().into_owned();
        self.stream(index, shown, 0, Some(size));
    }

    fn stream(&mut self, index: usize, shown: String, pinned: u64, done: Option<u64>) {
        let Ok((ours, theirs)) = UnixStream::pair().and_then(|(o, t)| o.set_nonblocking(true).map(|()| (o, t))) else {
            self.tracked[index].phase = Phase::Waiting;
            return;
        };
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        self.held(self.tracked[index].ino, |held| held.pinned = pinned);
        let t = &mut self.tracked[index];
        t.phase = Phase::Streaming(Box::new(Stream {
            id,
            socket: ours,
            theirs: Some(theirs),
            shown,
            born: Instant::now(),
            announced: None,
            free: false,
            sent: 0,
            pending: Vec::new(),
            pending_at: 0,
            punched: 0,
            pinned,
            hasher: Sha256::new(),
            received: false,
            closing: None,
            done,
        }));
    }

    /// The person's download fails; the rest of the file is discarded.
    fn fail(&mut self, index: usize) {
        if let Phase::Streaming(s) = &self.tracked[index].phase {
            self.files.fail_download(s.id);
        }
        self.discard(index);
    }

    /// Stops following a file (it no longer has its name).
    fn forget(&mut self, index: usize) {
        if let Phase::Streaming(s) = &self.tracked[index].phase
            && s.done.is_none()
        {
            self.fail(index);
        }
        self.tracked[index].phase = Phase::Gone;
    }

    /// Removes a file from the directory, if its name still holds it, and forgets it. It is moved into the launcher's
    /// own unwatched directory first and removed there, so closing it later says nothing about a newer file that
    /// takes its name.
    fn remove(&mut self, index: usize) {
        let t = &mut self.tracked[index];
        // Out of the gate's view first, so its inode is never taken for a newer file's.
        self.gate.held.lock().unwrap().remove(&t.ino);
        let same = rustix::fs::statat(&self.dir, t.name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)
            .is_ok_and(|s| s.st_ino == t.ino);
        let aside = t.ino.to_string();
        if same && rustix::fs::renameat(&self.dir, t.name.as_c_str(), &self.aside, aside.as_str()).is_ok() {
            let _ = rustix::fs::unlinkat(&self.aside, aside.as_str(), AtFlags::empty());
        }
        t.phase = Phase::Gone;
    }

    /// One pass: streams what the display can take, discards, ages temporary files, and forgets what is gone.
    pub fn tick(&mut self) -> Result<(), String> {
        enum Act {
            Fail,
            Pump,
            Wait,
            Admit,
            Punch(u64),
            Nothing,
        }
        let mut index = 0;
        while index < self.tracked.len() {
            let t = &self.tracked[index];
            let (links, size) = t.file.metadata().map(|m| (m.nlink(), m.len())).unwrap_or((0, 0));
            let act = match &t.phase {
                Phase::Streaming(s)
                    if s.done.is_none() && (links == 0 || s.closing.is_some_and(|c| c.elapsed() >= CLOSE_WITHIN)) =>
                {
                    Act::Fail
                }
                Phase::Streaming(_) => Act::Pump,
                Phase::Temporary(at) if at.elapsed() >= TEMPORARY_FOR => Act::Wait,
                Phase::Other if t.since.elapsed() >= TEMPORARY_FOR => Act::Admit,
                Phase::Discarding => Act::Punch(size),
                _ => Act::Nothing,
            };
            match act {
                Act::Fail => self.fail(index),
                Act::Pump => self.pump(index)?,
                Act::Admit => self.admitted(self.tracked[index].ino),
                Act::Wait => {
                    self.tracked[index].phase = Phase::Waiting;
                    self.admitted(self.tracked[index].ino);
                }
                Act::Punch(size) => {
                    let flags = FallocateFlags::PUNCH_HOLE | FallocateFlags::KEEP_SIZE;
                    let _ = rustix::fs::fallocate(&*self.tracked[index].file, flags, 0, size);
                }
                Act::Nothing => {}
            }
            let t = &mut self.tracked[index];
            if links == 0 && !matches!(&t.phase, Phase::Streaming(_)) {
                t.phase = Phase::Gone;
            }
            index += 1;
        }
        let mut held = self.gate.held.lock().unwrap();
        self.tracked.retain(|t| {
            let keep = !matches!(t.phase, Phase::Gone);
            if !keep && held.get(&t.ino).is_some_and(|h| Arc::ptr_eq(&h.file, &t.file)) {
                held.remove(&t.ino);
            }
            keep
        });
        Ok(())
    }

    /// Announces stream `index` to the display once its size is settled: a complete file's own, the one Chromium said
    /// (matched to exactly one of its downloads), or none after [`SIZE_WITHIN`]. False while it waits.
    fn announce(&mut self, index: usize) -> bool {
        let t = &mut self.tracked[index];
        let Phase::Streaming(s) = &mut t.phase else { return false };
        if s.announced.is_some() {
            if !s.free && s.announced.is_some_and(|at| at.elapsed() >= UNRECEIVED_FOR) {
                s.free = true;
                let ino = t.ino;
                self.admitted(ino);
            }
            return true;
        }
        let size = match s.done {
            Some(size) => Some(size),
            None => match self.sizes.lookup(&s.shown) {
                Lookup::Size(size) => size,
                Lookup::Wait if s.born.elapsed() < SIZE_WITHIN => return false,
                Lookup::Wait => None,
            },
        };
        let Some(theirs) = s.theirs.take() else { return false };
        let (hi, lo) = size.map_or((0, 0), |size| ((size >> 32) as u32, size as u32));
        self.files.start_download(s.id, theirs.as_fd(), s.shown.clone(), u32::from(size.is_some()), hi, lo);
        s.announced = Some(Instant::now());
        true
    }

    /// Hands the display what it can take of stream `index`.
    fn pump(&mut self, index: usize) -> Result<(), String> {
        if !self.announce(index) {
            return Ok(());
        }
        loop {
            let t = &mut self.tracked[index];
            let file = t.file.clone();
            let Phase::Streaming(s) = &mut t.phase else { return Ok(()) };
            if !s.pending.is_empty() {
                match s.socket.write(&s.pending[s.pending_at..]) {
                    Ok(n) => {
                        if s.sent < s.pinned {
                            let upto = (s.pinned - s.sent).min(n as u64) as usize;
                            s.hasher.update(&s.pending[s.pending_at..s.pending_at + upto]);
                        }
                        s.pending_at += n;
                        if s.pending_at == s.pending.len() {
                            s.pending.clear();
                            s.pending_at = 0;
                        }
                        s.sent += n as u64;
                        if s.received {
                            self.gate.drained(n as u64);
                        }
                        // Handed on: those whole pages leave the tmpfs (the pinned prefix stays).
                        let from = s.punched.max(s.pinned.next_multiple_of(4096));
                        let to = s.sent & !4095;
                        if to > from {
                            let flags = FallocateFlags::PUNCH_HOLE | FallocateFlags::KEEP_SIZE;
                            let _ = rustix::fs::fallocate(&*file, flags, from, to - from);
                            s.punched = to;
                        }
                        continue;
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(()),
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => {
                        // The display closed its end.
                        self.discard(index);
                        return Ok(());
                    }
                }
            }
            if let Some(size) = s.done
                && s.sent >= size
            {
                return self.finish(index);
            }
            if s.closing.is_some() && s.done.is_none() {
                // Frozen until the file reaches its plain name.
                return Ok(());
            }
            let limit = s.done.unwrap_or_else(|| file.metadata().map(|m| m.len()).unwrap_or(0));
            if s.sent >= limit {
                return Ok(());
            }
            let mut chunk = vec![0u8; ((limit - s.sent) as usize).min(STEP)];
            let n = match file.read_at(&mut chunk, s.sent) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(_) => {
                    self.fail(index);
                    return Ok(());
                }
            };
            chunk.truncate(n);
            // Before these bytes go on, every event queued so far: a later generation's bytes come with an earlier
            // close already queued.
            self.events()?;
            let Phase::Streaming(s) = &mut self.tracked[index].phase else { return Ok(()) };
            if s.closing.is_some() && s.done.is_none() {
                return Ok(());
            }
            s.pending = chunk;
        }
    }

    /// Every byte was handed on: the prefix is checked against the complete file, the socket closed, the display told,
    /// and the file removed.
    fn finish(&mut self, index: usize) -> Result<(), String> {
        let t = &mut self.tracked[index];
        let Phase::Streaming(s) = std::mem::replace(&mut t.phase, Phase::Discarding) else { return Ok(()) };
        let Stream { id, socket, pinned, hasher, done, .. } = *s;
        let mut prefix = vec![0u8; pinned as usize];
        let same = t.file.read_exact_at(&mut prefix, 0).is_ok() && Sha256::digest(&prefix) == hasher.finalize();
        drop(socket);
        if same {
            let size = done.unwrap_or(0);
            self.files.end_download(id, (size >> 32) as u32, size as u32);
        } else {
            eprintln!("browser-launch: downloads: a download's beginning changed after it was handed on; it fails");
            self.files.fail_download(id);
        }
        self.remove(index);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_page_is_whole_only_once_its_closing_boundary_is_written() {
        let page = "From: <Saved by Blink>\r\nContent-Type: multipart/related;\r\n\ttype=\"text/html\";\r\n\
            \tboundary=\"----MultipartBoundary--x1----\"\r\n\r\n------MultipartBoundary--x1----\r\n\
            Content-Type: text/html\r\n\r\n<p>hello</p>\r\n";
        let whole = format!("{page}------MultipartBoundary--x1------\r\n");
        // A title is copied into Subject: one naming a boundary of its own changes nothing.
        let titled =
            page.replace("From: <Saved by Blink>\r\n", "From: <Saved by Blink>\r\nSubject: a boundary=\"evil\"\r\n");
        let whole_titled = format!("{titled}------MultipartBoundary--x1------\r\n");
        let cut_titled = format!("{titled}--evil--\r\n");
        let cases = [
            (whole.as_str(), true),
            (whole_titled.as_str(), true),
            (page, false),
            (cut_titled.as_str(), false),
            ("", false),
            ("------MultipartBoundary--x1------", false),
            // The closing delimiter must be a line of its own.
            (&*format!("{page}x------MultipartBoundary--x1------"), false),
        ];
        for (i, (text, expected)) in cases.into_iter().enumerate() {
            let path = std::env::temp_dir().join(format!("mhtml-{}-{i}", std::process::id()));
            std::fs::write(&path, text).unwrap();
            assert_eq!(whole_mhtml(&File::open(&path).unwrap()), expected, "{text:?}");
            std::fs::remove_file(path).unwrap();
        }
    }
}
