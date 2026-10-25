//! A saved profile in the browser's container: the launcher's two profile commands, which the controller runs as this
//! container's uid (`controller/src/template.rs`, `Exec`) with their standard streams relayed to the admin, framed as
//! `display_protocol::profile` says.
//!
//! `profile-restore`, before Chrome starts, unpacks the archive the admin streams into a directory of its own, then
//! moves it whole into place as Chrome's user data directory, which the launcher waits for (`BROWSER_PROFILE`): Chrome
//! never sees part of one. Pages write the profile, so the archive is untrusted: only directories and regular files
//! are made, each created exclusively and never through a link (none is ever made), at a relative path of plain names
//! (no `..`, `.`, or empty name, no leading `/`), each directory before what it holds, within [`MAX_ENTRIES`],
//! [`MAX_PATH`] and [`MAX_DEPTH`], and its files within `MAX_ARCHIVE` together; anything else fails the restore, and
//! the session with it.
//!
//! `profile-save`, at the session's end, asks the launcher to close Chrome (`Browser.close` on its DevTools pipe, as a
//! person quitting it, which writes its cookies and storage out) and waits until it did, then streams the profile as a
//! tar archive, without what [`LEFT_OUT`] names: Chrome's caches, which it fills again, its crash reports, and the
//! previous Chrome's locks. A Chrome that does not close cleanly in time is killed and nothing is saved, so the admin
//! keeps the archive it had.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use display_protocol::profile::{Frame, HEADER, MAX_ARCHIVE, MAX_DATA, failure};
use rustix::fs::{Mode, OFlags};

use crate::cdp::Close;

/// Chrome's user data directory, on the profile's tmpfs.
pub const PROFILE: &str = "/home/browser/profile";
/// Where a restore unpacks, beside it on the same tmpfs, before it moves into place.
const RESTORING: &str = "/home/browser/profile.restoring";
/// How long the launcher waits for a restore: the admin starts it as soon as the session's display is up.
pub const RESTORE_WAIT: Duration = Duration::from_secs(120);
/// A save's request to the launcher, and the launcher's answer once Chrome is gone: `clean`, or `unclean` when it had
/// to be killed. Files the launcher polls for (it polls its children anyway), in the container's private `/tmp`.
pub const SAVE_REQUEST: &str = "/tmp/profile-save";
pub const CLOSED: &str = "/tmp/profile-closed";
/// How long Chrome may take to close cleanly once asked.
pub const CLOSE_WAIT: Duration = Duration::from_secs(20);
/// Files and directories in one archive.
pub const MAX_ENTRIES: usize = 100_000;
/// The longest path in an archive, in bytes, and the most names in one.
pub const MAX_PATH: usize = 1024;
pub const MAX_DEPTH: usize = 32;
/// What an archive leaves out, by path inside the user data directory; `*` stands for any one name (a profile's own
/// directory, `Default` or `Profile 1`). Chrome fills its caches again (a site's Cache API storage, which its pages
/// fetch again, included), the crash reports are this machine's, and the locks name a Chrome that is gone. The service
/// workers' scripts stay, with the registrations that name them.
const LEFT_OUT: &[&str] = &[
    "SingletonLock",
    "SingletonSocket",
    "SingletonCookie",
    "BrowserMetrics",
    "BrowserMetrics-spare.pma",
    "Crashpad",
    "Crash Reports",
    "ShaderCache",
    "GrShaderCache",
    "GraphiteDawnCache",
    "GPUPersistentCache",
    "component_crx_cache",
    "extensions_crx_cache",
    "*/Cache",
    "*/Code Cache",
    "*/GPUCache",
    "*/DawnCache",
    "*/DawnGraphiteCache",
    "*/DawnWebGPUCache",
    "*/Service Worker/CacheStorage",
];

/// Whether an archive leaves out `path` (relative to the user data directory, and everything below it).
fn left_out(path: &Path) -> bool {
    let names: Vec<&OsStr> = path.iter().collect();
    LEFT_OUT.iter().any(|pattern| {
        let pattern: Vec<&str> = pattern.split('/').collect();
        pattern.len() == names.len()
            && pattern.iter().zip(&names).all(|(want, name)| *want == "*" || OsStr::new(want) == *name)
    })
}

/// `name`, an archive's path, if it is a relative path of plain names within the bounds; a directory's may end in `/`.
fn checked(name: &[u8]) -> Result<PathBuf, String> {
    let shown = String::from_utf8_lossy(name);
    let trimmed = name.strip_suffix(b"/").unwrap_or(name);
    if trimmed.is_empty() || trimmed.len() > MAX_PATH {
        return Err(format!("the archive holds a path of {} bytes", trimmed.len()));
    }
    let names: Vec<&[u8]> = trimmed.split(|b| *b == b'/').collect();
    if names.len() > MAX_DEPTH {
        return Err(format!("the archive holds a path {} names deep", names.len()));
    }
    if names.iter().any(|n| n.is_empty() || *n == b"." || *n == b".." || n.contains(&0)) {
        return Err(format!("the archive holds a path outside the profile: {shown:?}"));
    }
    Ok(PathBuf::from(OsStr::from_bytes(trimmed)))
}

/// The archive's bytes, from the admin's data frames until its end frame; anything else is an error.
struct Incoming<R> {
    input: R,
    /// What is left of the current data frame.
    left: usize,
    ended: bool,
    total: u64,
}

impl<R: Read> Incoming<R> {
    fn new(input: R) -> Incoming<R> {
        Incoming { input, left: 0, ended: false, total: 0 }
    }

    /// After the archive's last entry: what is left (the rest of tar's end, zeros) up to the end frame.
    fn finish(mut self) -> Result<(), String> {
        let mut rest = Vec::new();
        (&mut self).take(MAX_DATA as u64 + 1).read_to_end(&mut rest).map_err(|e| format!("the archive: {e}"))?;
        if rest.len() > MAX_DATA || rest.iter().any(|b| *b != 0) || !self.ended {
            return Err("the archive goes on after its end".into());
        }
        Ok(())
    }
}

impl<R: Read> Read for Incoming<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.left == 0 {
            if self.ended || buf.is_empty() {
                return Ok(0);
            }
            let mut header = [0u8; HEADER];
            self.input.read_exact(&mut header)?;
            match Frame::parse(header).map_err(std::io::Error::other)? {
                Frame::Data(length) => {
                    self.total += length as u64;
                    if self.total > MAX_ARCHIVE {
                        return Err(std::io::Error::other(format!("the archive is longer than {MAX_ARCHIVE} bytes")));
                    }
                    self.left = length;
                }
                Frame::End => self.ended = true,
                Frame::Failed(length) => {
                    let mut why = vec![0u8; length];
                    self.input.read_exact(&mut why)?;
                    let why = String::from_utf8_lossy(&why);
                    return Err(std::io::Error::other(format!("the admin stopped sending it: {why}")));
                }
            }
        }
        let wanted = buf.len().min(self.left);
        let read = self.input.read(&mut buf[..wanted])?;
        if read == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        self.left -= read;
        Ok(read)
    }
}

/// The archive, written as data frames of [`MAX_DATA`] bytes; [`Outgoing::end`] sends the end frame.
struct Outgoing<W: Write> {
    output: W,
    buffer: Vec<u8>,
}

impl<W: Write> Outgoing<W> {
    fn new(output: W) -> Outgoing<W> {
        Outgoing { output, buffer: Vec::with_capacity(MAX_DATA) }
    }

    fn send(&mut self) -> std::io::Result<()> {
        if !self.buffer.is_empty() {
            self.output.write_all(&Frame::Data(self.buffer.len()).header())?;
            self.output.write_all(&self.buffer)?;
            self.buffer.clear();
        }
        Ok(())
    }

    /// Sends the end frame, and gives the output back.
    fn end(mut self) -> std::io::Result<W> {
        self.send()?;
        self.output.write_all(&Frame::End.header())?;
        self.output.flush()?;
        Ok(self.output)
    }
}

impl<W: Write> Write for Outgoing<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let taken = bytes.len().min(MAX_DATA - self.buffer.len());
        self.buffer.extend_from_slice(&bytes[..taken]);
        if self.buffer.len() == MAX_DATA {
            self.send()?;
        }
        Ok(taken)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.send()?;
        self.output.flush()
    }
}

/// Says `why` the archive stops, as the last frame on `output`.
fn fail(mut output: impl Write, why: &str) {
    let body = failure(why);
    let _ = output.write_all(&Frame::Failed(body.len()).header());
    let _ = output.write_all(body);
    let _ = output.flush();
}

/// Unpacks the tar archive `input` into `into`, an empty directory, as the module's rules say.
fn unpack(input: impl Read, into: &Path) -> Result<(), String> {
    let mut archive = tar::Archive::new(input);
    let (mut entries, mut total) = (0usize, 0u64);
    for entry in archive.entries().map_err(|e| format!("the archive: {e}"))? {
        let mut entry = entry.map_err(|e| format!("the archive: {e}"))?;
        entries += 1;
        if entries > MAX_ENTRIES {
            return Err(format!("the archive holds more than {MAX_ENTRIES} files"));
        }
        let name = entry.path_bytes().into_owned();
        let path = into.join(checked(&name)?);
        match entry.header().entry_type() {
            tar::EntryType::Directory => std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .map_err(|e| format!("{}: {e}", String::from_utf8_lossy(&name)))?,
            tar::EntryType::Regular => {
                let size = entry.size();
                total += size;
                if total > MAX_ARCHIVE {
                    return Err(format!("the archive's files hold more than {MAX_ARCHIVE} bytes"));
                }
                let file = crate::create_new(&path).map_err(|e| format!("{}: {e}", String::from_utf8_lossy(&name)))?;
                let copied = std::io::copy(&mut (&mut entry).take(size), &mut std::fs::File::from(file))
                    .map_err(|e| format!("{}: {e}", String::from_utf8_lossy(&name)))?;
                if copied != size {
                    return Err(format!("{} is cut short", String::from_utf8_lossy(&name)));
                }
            }
            other => {
                return Err(format!(
                    "the archive holds {other:?}, not a file or a directory: {:?}",
                    String::from_utf8_lossy(&name)
                ));
            }
        }
    }
    Ok(())
}

/// A file's bytes, exactly `left` of them: one that turned shorter since its size was read fails the archive rather
/// than leaving it misaligned.
struct Exact<R> {
    inner: R,
    left: u64,
}

impl<R: Read> Read for Exact<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 || buf.is_empty() {
            return Ok(0);
        }
        let wanted = buf.len().min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let read = self.inner.read(&mut buf[..wanted])?;
        if read == 0 {
            return Err(std::io::Error::other("a file turned shorter while it was saved"));
        }
        self.left -= read as u64;
        Ok(read)
    }
}

/// Packs the user data directory `from` as a tar archive on `output`: its directories and regular files, in name
/// order, without what [`LEFT_OUT`] names; links, sockets and FIFOs are skipped. One it could not restore fails here.
fn pack(from: &Path, output: impl Write) -> Result<(), String> {
    let mut builder = tar::Builder::new(output);
    let mut entries = 0;
    walk(&mut builder, from, Path::new(""), &mut entries)?;
    builder.into_inner().map_err(|e| format!("the archive: {e}"))?;
    Ok(())
}

fn walk<W: Write>(builder: &mut tar::Builder<W>, dir: &Path, at: &Path, entries: &mut usize) -> Result<(), String> {
    let listed = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut names = listed.map(|entry| entry.map(|e| e.file_name())).collect::<Result<Vec<_>, _>>();
    let names = names.as_mut().map_err(|e| format!("{}: {e}", dir.display()))?;
    names.sort();
    for name in names.iter() {
        let relative = at.join(name);
        if left_out(&relative) {
            continue;
        }
        let path = dir.join(name);
        let kind = std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?.file_type();
        if !kind.is_dir() && !kind.is_file() {
            continue;
        }
        *entries += 1;
        if *entries > MAX_ENTRIES {
            return Err(format!("the profile holds more than {MAX_ENTRIES} files"));
        }
        checked(relative.as_os_str().as_bytes())?;
        let mut header = tar::Header::new_gnu();
        if kind.is_dir() {
            header.set_entry_type(tar::EntryType::Directory);
            header.set_mode(0o700);
            header.set_size(0);
            builder.append_data(&mut header, &relative, std::io::empty()).map_err(|e| format!("the archive: {e}"))?;
            walk(builder, &path, &relative, entries)?;
        } else {
            let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let file = rustix::fs::open(&path, flags, Mode::empty()).map_err(|e| format!("{}: {e}", path.display()))?;
            let file = std::fs::File::from(file);
            let size = file.metadata().map_err(|e| format!("{}: {e}", path.display()))?.len();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_mode(0o600);
            header.set_size(size);
            builder
                .append_data(&mut header, &relative, Exact { inner: file, left: size })
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Makes the process non-dumpable (a profile command before it holds anything of the profile, the launcher before
/// Chrome starts): the kernel makes no core dump of it, and no other process of its uid may read its memory.
pub fn non_dumpable() -> Result<(), String> {
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)
        .map_err(|e| format!("cannot make itself non-dumpable: {e}"))
}

/// `browser-launch profile-restore`: unpacks the archive on standard input into the profile, and answers on standard
/// output with an end frame once it is in place, or a failure.
pub fn restore() -> ExitCode {
    let restored = non_dumpable().and_then(|()| {
        // Created afresh: a second restore of the same session finds it there and stops.
        std::fs::DirBuilder::new().mode(0o700).create(RESTORING).map_err(|e| format!("{RESTORING}: {e}"))?;
        let mut incoming = Incoming::new(std::io::stdin().lock());
        unpack(&mut incoming, Path::new(RESTORING))?;
        incoming.finish()?;
        // Never over a profile already there.
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            RESTORING,
            rustix::fs::CWD,
            PROFILE,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|e| format!("{PROFILE}: {e}"))
    });
    let mut output = std::io::stdout().lock();
    match restored {
        Ok(()) => {
            let _ = output.write_all(&Frame::End.header()).and_then(|()| output.flush());
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("browser-launch: profile-restore: {why}");
            fail(output, &why);
            ExitCode::FAILURE
        }
    }
}

/// `browser-launch profile-save`: has the launcher close Chrome, then streams the profile on standard output, ending
/// with an end frame, or a failure.
pub fn save() -> ExitCode {
    let mut outgoing = Outgoing::new(std::io::stdout().lock());
    let saved = non_dumpable().and_then(|()| close_chrome()).and_then(|()| pack(Path::new(PROFILE), &mut outgoing));
    let sent = match saved {
        Ok(()) => outgoing.end().map(drop).map_err(|e| format!("sending it: {e}")),
        Err(why) => Err(why),
    };
    match sent {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("browser-launch: profile-save: {why}");
            fail(std::io::stdout().lock(), &why);
            ExitCode::FAILURE
        }
    }
}

/// Asks the launcher to close Chrome, and waits for its answer: a clean close, or why there was none.
fn close_chrome() -> Result<(), String> {
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(SAVE_REQUEST)
        .map_err(|e| format!("{SAVE_REQUEST}: {e}"))?;
    let deadline = Instant::now() + CLOSE_WAIT + Duration::from_secs(5);
    loop {
        match std::fs::read_to_string(CLOSED).as_deref() {
            Ok("clean") => return Ok(()),
            Ok("unclean") => {
                return Err("Chrome did not close cleanly in time, so its profile may be incomplete".into());
            }
            _ if Instant::now() > deadline => return Err("the launcher did not close Chrome".into()),
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// The launcher's side of a save: once [`SAVE_REQUEST`] is there, asks Chrome to close, and answers in [`CLOSED`] when
/// it did, or once it had to be killed after [`CLOSE_WAIT`]. The launcher calls it on every turn of its loop, in a
/// session started from a profile; it answers whether Chrome is closed for good.
pub struct Closing {
    request: PathBuf,
    answer: PathBuf,
    asked: Option<Instant>,
    /// `Browser.close` is on its way to Chrome.
    sent: bool,
    /// Chrome had no DevTools pipe left and was sent SIGTERM, which it takes as the session ending: its browser
    /// process exits with status 0 within milliseconds without writing out the cookies its store still held (they are
    /// committed in batches, about every 30 seconds), and only its orphaned network process may write them later,
    /// after the profile was packed. So it never closed cleanly.
    forced: bool,
    closed: bool,
}

impl Default for Closing {
    fn default() -> Closing {
        Closing::at(SAVE_REQUEST, CLOSED)
    }
}

impl Closing {
    /// A save asked for at `request` and answered at `answer`.
    fn at(request: impl Into<PathBuf>, answer: impl Into<PathBuf>) -> Closing {
        Closing {
            request: request.into(),
            answer: answer.into(),
            asked: None,
            sent: false,
            forced: false,
            closed: false,
        }
    }

    /// One turn: `chromium` is the Chrome running now, and `close` asks it to close over its DevTools pipe.
    pub fn turn(&mut self, chromium: &mut std::process::Child, close: impl FnOnce() -> Close) -> bool {
        if self.closed {
            return true;
        }
        let asked = match self.asked {
            Some(asked) => asked,
            None if self.request.exists() => *self.asked.insert(Instant::now()),
            None => return false,
        };
        // Closed as a person quitting it is, which writes the profile out before the browser process exits, asked
        // again on the next turn while its pipe's queue is full; SIGTERM ends it too, but with the last seconds'
        // cookies unwritten.
        if !(self.sent || self.forced) {
            match close() {
                Close::Sent => self.sent = true,
                Close::Busy => {}
                Close::Gone => {
                    self.forced = true;
                    let pid = rustix::process::Pid::from_child(chromium);
                    let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
                }
            }
        }
        // Chrome exits with status 0 once its shutdown is done; any other end (a crash, a signal) may have left its
        // files half written, so nothing is saved.
        let answer = match chromium.try_wait() {
            Ok(Some(status)) if status.success() && !self.forced => "clean",
            Ok(Some(_)) => "unclean",
            _ if asked.elapsed() > CLOSE_WAIT => {
                let _ = chromium.kill();
                let _ = chromium.wait();
                "unclean"
            }
            _ => return true,
        };
        if let Err(e) = std::fs::write(&self.answer, answer) {
            eprintln!("browser-launch: {}: {e}", self.answer.display());
        }
        eprintln!("browser-launch: chromium closed for the profile's save ({answer})");
        self.closed = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory for one test, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!("profile-{name}-{}-{:?}", std::process::id(), Instant::now()));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// `archive` framed as the admin sends it: in data frames, then the end frame.
    fn framed(archive: &[u8]) -> Vec<u8> {
        let mut out = Outgoing::new(Vec::new());
        out.write_all(archive).unwrap();
        out.end().unwrap()
    }

    /// Data frames of zeros, without end.
    struct Endless(usize);

    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let frame = HEADER + MAX_DATA;
            let at = self.0 % frame;
            let n = if at < HEADER {
                let header = Frame::Data(MAX_DATA).header();
                let n = buf.len().min(HEADER - at);
                buf[..n].copy_from_slice(&header[at..at + n]);
                n
            } else {
                let n = buf.len().min(frame - at);
                buf[..n].fill(0);
                n
            };
            self.0 += n;
            Ok(n)
        }
    }

    fn restore_into(archive: &[u8], into: &Path) -> Result<(), String> {
        let frames = framed(archive);
        let mut incoming = Incoming::new(&frames[..]);
        unpack(&mut incoming, into)?;
        incoming.finish()
    }

    /// One raw entry of [`crafted`]: its type, path, and contents.
    type Raw<'a> = (tar::EntryType, &'a [u8], &'a [u8]);

    /// A tar archive of raw entries, written header by header, as a crafted one would be.
    fn crafted(entries: &[Raw]) -> Vec<u8> {
        let mut out = Vec::new();
        for (kind, path, data) in entries {
            let mut header = tar::Header::new_old();
            header.as_old_mut().name[..path.len()].copy_from_slice(path);
            header.set_entry_type(*kind);
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            if *kind == tar::EntryType::Symlink || *kind == tar::EntryType::Link {
                header.set_link_name("/etc/passwd").unwrap();
                header.set_size(0);
            }
            header.set_cksum();
            out.extend_from_slice(header.as_bytes());
            if header.size().unwrap() > 0 {
                out.extend_from_slice(data);
                out.resize(out.len().next_multiple_of(512), 0);
            }
        }
        out.extend_from_slice(&[0; 1024]);
        out
    }

    fn tree(dir: &Path) -> Vec<(String, Option<Vec<u8>>)> {
        let mut found = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(at) = stack.pop() {
            for entry in std::fs::read_dir(&at).unwrap() {
                let path = entry.unwrap().path();
                let shown = path.strip_prefix(dir).unwrap().display().to_string();
                if path.is_dir() {
                    found.push((shown, None));
                    stack.push(path);
                } else {
                    found.push((shown, Some(std::fs::read(&path).unwrap())));
                }
            }
        }
        found.sort();
        found
    }

    #[test]
    fn a_profile_packs_without_its_caches_and_locks_and_unpacks_the_same() {
        let scratch = Scratch::new("roundtrip");
        let from = scratch.0.join("from");
        let write = |path: &str, bytes: &[u8]| {
            let path = from.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        write("Local State", b"{}");
        write("Default/Cookies", &vec![42; 3 * MAX_DATA + 5]);
        write("Default/Local Storage/leveldb/000003.log", b"session=1");
        write(&format!("Default/IndexedDB/{}/blob", "a".repeat(200)), b"long names go too");
        write("Default/Cache/Cache_Data/index", b"cached");
        write("Default/Code Cache/js/index", b"cached");
        write("Default/Service Worker/CacheStorage/x", b"cached");
        write("Default/Service Worker/Database/CURRENT", b"kept");
        write("ShaderCache/data_0", b"cached");
        std::fs::create_dir_all(from.join("Default/empty")).unwrap();
        std::os::unix::fs::symlink("example-1234", from.join("SingletonLock")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", from.join("Default/elsewhere")).unwrap();
        let mut archive = Vec::new();
        pack(&from, &mut archive).unwrap();
        let into = scratch.0.join("into");
        std::fs::create_dir(&into).unwrap();
        restore_into(&archive, &into).unwrap();
        let unpacked = tree(&into);
        let names: Vec<&str> = unpacked.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&"Default/Cookies") && names.contains(&"Default/empty"), "{names:?}");
        assert!(names.contains(&"Default/Service Worker/Database/CURRENT"), "{names:?}");
        for gone in ["Default/Cache", "Default/Code Cache", "Default/Service Worker/CacheStorage", "ShaderCache"] {
            assert!(!names.contains(&gone), "{gone} is a cache");
        }
        assert!(!names.contains(&"SingletonLock") && !names.contains(&"Default/elsewhere"), "links are skipped");
        let cookies = unpacked.iter().find(|(name, _)| name == "Default/Cookies").unwrap();
        assert_eq!(cookies.1.as_deref(), Some(&vec![42; 3 * MAX_DATA + 5][..]));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(into.join("Default/Cookies")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "private whatever the archive says");
    }

    #[test]
    fn an_archive_reaching_outside_the_profile_or_holding_a_link_is_refused() {
        use tar::EntryType::{Block, Char, Directory, Fifo, Link, Regular, Symlink};
        let scratch = Scratch::new("hostile");
        let cases: Vec<(&str, Vec<Raw>)> = vec![
            ("a parent", vec![(Regular, b"../escaped", b"x")]),
            ("a parent inside", vec![(Directory, b"Default/", b""), (Regular, b"Default/../../escaped", b"x")]),
            ("an absolute path", vec![(Regular, b"/tmp/escaped", b"x")]),
            ("a dot", vec![(Regular, b"./Cookies", b"x")]),
            ("an empty name", vec![(Directory, b"Default/", b""), (Regular, b"Default//Cookies", b"x")]),
            ("a symbolic link", vec![(Symlink, b"Default", b"")]),
            ("a hard link", vec![(Link, b"Cookies", b"")]),
            ("a device", vec![(Char, b"null", b"")]),
            ("a block device", vec![(Block, b"disk", b"")]),
            ("a FIFO", vec![(Fifo, b"pipe", b"")]),
            ("a file twice", vec![(Regular, b"Cookies", b"a"), (Regular, b"Cookies", b"b")]),
            ("a file before its directory", vec![(Regular, b"Default/Cookies", b"x")]),
            ("a directory over a file", vec![(Regular, b"Default", b"x"), (Directory, b"Default/", b"")]),
        ];
        for (what, entries) in cases {
            let into = scratch.0.join(what.replace(' ', "-"));
            std::fs::create_dir(&into).unwrap();
            assert!(restore_into(&crafted(&entries), &into).is_err(), "{what}");
            assert!(!scratch.0.join("escaped").exists() && !Path::new("/tmp/escaped").exists(), "{what}");
        }
        let into = scratch.0.join("plain");
        std::fs::create_dir(&into).unwrap();
        let plain: Vec<Raw> = vec![(Directory, b"Default/", b""), (Regular, b"Default/Cookies", b"yum")];
        assert_eq!(restore_into(&crafted(&plain), &into), Ok(()), "the same writer's plain archive unpacks");
        assert_eq!(std::fs::read(into.join("Default/Cookies")).unwrap(), b"yum");
    }

    #[test]
    fn a_profile_never_saved_restores_as_an_empty_one() {
        let scratch = Scratch::new("empty");
        assert_eq!(restore_into(&[], &scratch.0), Ok(()), "no bytes, then the end frame");
        assert!(tree(&scratch.0).is_empty());
        let mut packed = Vec::new();
        let empty = scratch.0.join("empty");
        std::fs::create_dir(&empty).unwrap();
        pack(&empty, &mut packed).unwrap();
        let into = scratch.0.join("into");
        std::fs::create_dir(&into).unwrap();
        assert_eq!(restore_into(&packed, &into), Ok(()), "and an empty one packed");
    }

    #[test]
    fn paths_are_plain_names_within_the_bounds() {
        assert_eq!(checked(b"Default/Cookies").unwrap(), Path::new("Default/Cookies"));
        assert_eq!(checked(b"Default/").unwrap(), Path::new("Default"));
        let deep = vec!["a"; MAX_DEPTH].join("/");
        assert!(checked(deep.as_bytes()).is_ok());
        assert!(checked(format!("{deep}/a").as_bytes()).is_err(), "one name too deep");
        assert!(checked("a".repeat(MAX_PATH).as_bytes()).is_ok());
        assert!(checked("a".repeat(MAX_PATH + 1).as_bytes()).is_err(), "one byte too long");
        for bad in [&b""[..], b"/", b"..", b"a/..", b"a/./b", b"a\0b", b"//a"] {
            assert!(checked(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn an_archive_cut_failed_or_past_its_bound_is_refused() {
        let scratch = Scratch::new("bounds");
        let archive = crafted(&[(tar::EntryType::Regular, b"Cookies", b"yum")]);
        let mut cut = framed(&archive);
        cut.truncate(cut.len() - HEADER);
        let into = scratch.0.join("cut");
        std::fs::create_dir(&into).unwrap();
        let mut incoming = Incoming::new(&cut[..]);
        let unpacked = unpack(&mut incoming, &into).and_then(|()| incoming.finish());
        assert!(unpacked.is_err(), "no end frame: not whole");
        let mut failed = archive[..512].to_vec();
        failed = framed(&failed);
        failed.truncate(failed.len() - HEADER);
        failed.extend_from_slice(&Frame::Failed(4).header());
        failed.extend_from_slice(b"oops");
        let into = scratch.0.join("failed");
        std::fs::create_dir(&into).unwrap();
        let why = unpack(Incoming::new(&failed[..]), &into).unwrap_err();
        assert!(why.contains("oops"), "{why}");
        // Past the bound, counted as the frames arrive: refused before the bytes are taken.
        let mut header = tar::Header::new_gnu();
        header.set_path("Cookies").unwrap();
        header.set_size(MAX_ARCHIVE + 1);
        header.set_cksum();
        let into = scratch.0.join("long");
        std::fs::create_dir(&into).unwrap();
        let mut frames = framed(header.as_bytes());
        frames.truncate(frames.len() - HEADER);
        let why = unpack(Incoming::new(&frames[..]), &into).unwrap_err();
        assert!(why.contains("more than"), "{why}");
        let taken = std::io::copy(&mut Incoming::new(Endless(0)), &mut std::io::sink());
        assert!(taken.is_err(), "the stream itself is bounded");
        // Something after the archive's end.
        let mut trailing = archive.clone();
        trailing.extend_from_slice(b"more");
        let into = scratch.0.join("trailing");
        std::fs::create_dir(&into).unwrap();
        assert!(restore_into(&trailing, &into).is_err());
    }

    #[test]
    fn caches_and_locks_are_left_out_only_where_chrome_keeps_them() {
        for gone in ["SingletonLock", "Default/Cache", "Profile 1/Code Cache", "Default/Service Worker/CacheStorage"] {
            assert!(left_out(Path::new(gone)), "{gone}");
        }
        for kept in [
            "Default",
            "Default/Cookies",
            "Default/IndexedDB/Cache",
            "Cache",
            "Default/Service Worker",
            "Default/Service Worker/ScriptCache",
        ] {
            assert!(!left_out(Path::new(kept)), "{kept}");
        }
    }

    #[test]
    fn chrome_closed_over_its_pipe_is_clean_and_one_sent_sigterm_never_is() {
        let scratch = Scratch::new("closing");
        let (request, answer) = (scratch.0.join("save"), scratch.0.join("closed"));
        // Turns until the answer, with `chrome` (a shell program) as Chrome and `pipe` as what its pipe made of each
        // close asked for, the last one again for every later ask; the answer, and how many times the close was asked.
        let close = |chrome: &str, pipe: &[Close]| -> (String, usize) {
            let _ = std::fs::remove_file(&answer);
            let mut child = std::process::Command::new("sh").args(["-c", chrome]).spawn().unwrap();
            let mut closing = Closing::at(&request, &answer);
            let asked = std::cell::Cell::new(0);
            let end = Instant::now() + Duration::from_secs(10);
            while !answer.exists() && Instant::now() < end {
                assert!(closing.turn(&mut child, || {
                    asked.set(asked.get() + 1);
                    pipe[(asked.get() - 1).min(pipe.len() - 1)]
                }));
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = child.kill();
            let _ = child.wait();
            (std::fs::read_to_string(&answer).unwrap_or_default(), asked.get())
        };
        // Not asked yet: Chrome is left alone.
        let mut child = std::process::Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        assert!(!Closing::at(&request, &answer).turn(&mut child, || panic!("closed unasked")));
        child.wait().unwrap();
        std::fs::write(&request, "").unwrap();
        assert_eq!(close("sleep 0.2", &[Close::Sent]), ("clean".into(), 1));
        assert_eq!(close("sleep 0.2; exit 3", &[Close::Sent]), ("unclean".into(), 1));
        // A queue full for now: asked again on the next turns, until it takes the close.
        assert_eq!(close("sleep 0.2", &[Close::Busy, Close::Busy, Close::Sent]), ("clean".into(), 3));
        // No pipe: SIGTERM closes it, and even its status 0 then is no clean close.
        let exits_cleanly_on_term = "trap 'exit 0' TERM; while :; do sleep 0.05; done";
        assert_eq!(close(exits_cleanly_on_term, &[Close::Busy, Close::Gone]), ("unclean".into(), 2));
    }
}
