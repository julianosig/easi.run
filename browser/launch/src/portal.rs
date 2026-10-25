//! The file chooser portal (`org.freedesktop.portal.FileChooser`) on the browser's session bus: Chromium asks it for
//! files, and it asks the person holding control, through the display (`easi_files_v1.open_chooser`).
//!
//! The launcher owns the portal's name before Chromium starts, without allowing replacement, and the bus policy lets
//! only the browser's uid own or call it. Because the DevTools agent runs as that uid too, every caller must also be a
//! process of Chromium's: the bus attests the caller with a process descriptor, which pins its identity while the
//! launcher walks its parents up to the Chromium it started. `OpenFile` of files asks the person; `SaveFile`
//! (Chromium's "Save as", which its PDF viewer always uses) is answered at once with a new name in the download
//! directory, reserved there exclusively, so what Chromium saves (as a download, renamed into place) reaches the
//! person; choosing a directory and saving several files are refused. An answer goes to the caller that asked, on its
//! own request object, and `Close` on that object cancels it.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Arc, Mutex, OnceLock};

use wayland_client::Connection as Wayland;
use zbus::message::Header;
use zbus::zvariant::{OwnedObjectPath, Value};

use crate::DOWNLOADS;
use crate::files::protocol::easi_files_v1::EasiFilesV1;

const NAME: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
/// Choosers open at once (each waits for the person).
const MAX_OPEN: usize = 4;

/// A chooser waiting for the person: who asked, the request object to answer on, and the paths chosen so far.
struct Pending {
    caller: String,
    handle: String,
    paths: Vec<String>,
}

/// The Chromium running now: its process, and which one it is (a relaunch starts the next).
#[derive(Clone, Copy, Default)]
struct Browser {
    pid: Option<u32>,
    generation: u64,
}

/// What the portal and the files thread share.
pub struct Shared {
    files: EasiFilesV1,
    wayland: Wayland,
    bus: OnceLock<zbus::blocking::Connection>,
    /// The Chromium process the launcher started last; callers must descend from it.
    browser: Mutex<Browser>,
    pending: Mutex<HashMap<u32, Pending>>,
    next: Mutex<u32>,
    /// Answers to send right after their call returns, by one thread: (caller, request object, the file's URI).
    answers: Mutex<Option<std::sync::mpsc::SyncSender<(String, String, String)>>>,
    /// Names handed out for saves, with when, until the file arrives (see [`reserve_name`]).
    pub saves: Mutex<HashMap<String, std::time::Instant>>,
}

impl Shared {
    pub fn new(files: EasiFilesV1, wayland: Wayland) -> Arc<Shared> {
        Arc::new(Shared {
            files,
            wayland,
            bus: OnceLock::new(),
            browser: Mutex::new(Browser::default()),
            pending: Mutex::new(HashMap::new()),
            next: Mutex::new(0),
            answers: Mutex::new(None),
            saves: Mutex::new(HashMap::new()),
        })
    }

    /// A new Chromium runs as `pid`: callers must now descend from it, and the choosers the previous one opened are
    /// closed (the display stops asking the person; nobody is left to answer).
    pub fn started(&self, pid: u32) {
        let mut browser = self.browser.lock().unwrap();
        browser.pid = Some(pid);
        browser.generation += 1;
        let stale: Vec<(u32, Pending)> = self.pending.lock().unwrap().drain().collect();
        drop(browser);
        if stale.is_empty() {
            return;
        }
        for (chooser, _) in &stale {
            self.files.close_chooser(*chooser);
        }
        let _ = self.wayland.flush();
        if let Some(bus) = self.bus.get() {
            for (_, pending) in stale {
                let _ = bus.object_server().remove::<Request, _>(pending.handle.as_str());
            }
        }
    }

    /// The display sent a path the person chose for `chooser`.
    pub fn chosen(&self, chooser: u32, path: String) {
        if let Some(pending) = self.pending.lock().unwrap().get_mut(&chooser) {
            pending.paths.push(path);
        }
    }

    /// The display finished the chooser: answer with the chosen files, or cancelled.
    pub fn finish(&self, chooser: u32, done: bool) {
        let Some(pending) = self.pending.lock().unwrap().remove(&chooser) else { return };
        let Some(bus) = self.bus.get() else { return };
        let uris = if done { pending.paths.iter().map(|p| file_uri(p)).collect() } else { Vec::new() };
        respond(bus, &pending.caller, pending.handle.as_str(), uris);
        let _ = bus.object_server().remove::<Request, _>(pending.handle.as_str());
    }
}

/// Answers `caller`'s request at `handle`: with `uris`, or cancelled when there are none.
fn respond(bus: &zbus::blocking::Connection, caller: &str, handle: &str, uris: Vec<String>) {
    let mut results: HashMap<&str, Value<'_>> = HashMap::new();
    let code: u32 = if uris.is_empty() {
        1
    } else {
        results.insert("uris", Value::from(uris));
        0
    };
    let _ = bus.emit_signal(Some(caller), handle, "org.freedesktop.portal.Request", "Response", &(code, results));
}

/// A `file://` URI of an absolute path, every byte outside the unreserved set and `/` percent-encoded.
fn file_uri(path: &str) -> String {
    let mut uri = String::from("file://");
    for &b in path.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// Serves the portal on the session bus at `address`.
pub fn serve(shared: Arc<Shared>, address: &str) -> Result<(), String> {
    let bus = zbus::blocking::connection::Builder::address(address)
        .and_then(|b| b.serve_at(PATH, Portal { shared: shared.clone() }))
        .and_then(|b| b.name(NAME))
        .map(|b| b.allow_name_replacements(false).replace_existing_names(false))
        .and_then(|b| b.build())
        .map_err(|e| format!("file chooser portal: {e}"))?;
    let _ = shared.bus.set(bus);
    // Save answers go out from one thread, a few calls at most behind; a full queue refuses the save instead.
    let (send, receive) = std::sync::mpsc::sync_channel::<(String, String, String)>(MAX_SAVES);
    *shared.answers.lock().unwrap() = Some(send);
    let answering = shared.clone();
    std::thread::Builder::new()
        .name("portal-answers".into())
        .spawn(move || {
            for (caller, handle, uri) in receive {
                std::thread::sleep(std::time::Duration::from_millis(20));
                let Some(bus) = answering.bus.get() else { continue };
                respond(bus, &caller, &handle, vec![uri]);
            }
        })
        .map_err(|e| format!("file chooser portal: {e}"))?;
    Ok(())
}

struct Portal {
    shared: Arc<Shared>,
}

/// The request object a caller waits on; `Close` from that caller, and only that caller, cancels its chooser.
struct Request {
    shared: Arc<Shared>,
    chooser: u32,
    caller: String,
}

#[zbus::interface(name = "org.freedesktop.portal.Request")]
impl Request {
    fn close(&self, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        if header.sender().map(|s| s.as_str()) != Some(self.caller.as_str()) {
            return Err(refused("only the caller that asked may close its request"));
        }
        let Some(pending) = self.shared.pending.lock().unwrap().remove(&self.chooser) else { return Ok(()) };
        // The display stops asking the person.
        self.shared.files.close_chooser(self.chooser);
        let _ = self.shared.wayland.flush();
        // An object cannot remove itself while it answers a call: another thread does, right after.
        let shared = self.shared.clone();
        let _ = std::thread::Builder::new().name("portal-close".into()).spawn(move || {
            if let Some(bus) = shared.bus.get() {
                let _ = bus.object_server().remove::<Request, _>(pending.handle.as_str());
            }
        });
        Ok(())
    }
}

fn refused(why: &str) -> zbus::fdo::Error {
    zbus::fdo::Error::AccessDenied(why.to_owned())
}

/// The caller's unique name, its request object's path, and the Chromium generation it was admitted under, once the
/// bus attested the caller to be the running Chromium's.
async fn admitted(
    shared: &Shared,
    header: &Header<'_>,
    connection: &zbus::Connection,
    options: &HashMap<&str, Value<'_>>,
) -> zbus::fdo::Result<(String, String, u64)> {
    let caller = header.sender().ok_or_else(|| refused("no sender"))?.to_owned();
    let credentials =
        zbus::fdo::DBusProxy::new(connection).await?.get_connection_credentials(caller.clone().into()).await?;
    let process = credentials
        .process_fd()
        .and_then(|fd| std::os::fd::AsFd::as_fd(fd).try_clone_to_owned().ok())
        .ok_or_else(|| refused("the bus attested no process"))?;
    let browser = *shared.browser.lock().unwrap();
    let chromium = browser.pid.ok_or_else(|| refused("Chromium has not started"))?;
    if !descends_from(&process, chromium) {
        return Err(refused("only Chromium may ask for files"));
    }
    let token = match options.get("handle_token") {
        Some(Value::Str(token))
            if token.len() <= 64 && token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') =>
        {
            token.to_string()
        }
        _ => "chooser".to_owned(),
    };
    let handle = format!("{PATH}/request/{}/{token}", caller.trim_start_matches(':').replace('.', "_"));
    Ok((caller.to_string(), handle, browser.generation))
}

/// Longest saved name, in bytes, before a " (n)" that makes it new (the filesystem allows 255).
const MAX_SAVED_NAME: usize = 240;

/// Names handed out for saves at once; the oldest is forgotten first.
const MAX_SAVES: usize = 64;
/// A save not finished within this long is forgotten.
pub const SAVE_FOR: std::time::Duration = std::time::Duration::from_secs(600);

/// Picks a name for a file Chromium saves in `dir`: the suggested one without path separators or control characters, at
/// most [`MAX_SAVED_NAME`] bytes of UTF-8, numbered if a file or another pending save has it. It is created empty and
/// exclusively (Chromium's own save dialogs expect the chosen file to exist) and remembered in `saves` until the saved
/// file arrives; none while [`MAX_SAVES`] are pending.
fn reserve_name(dir: &str, suggested: &str, saves: &mut HashMap<String, std::time::Instant>) -> Option<String> {
    let mut clean = String::new();
    for c in suggested.chars() {
        let c = if c == '/' || c.is_control() { '_' } else { c };
        if clean.len() + c.len_utf8() > MAX_SAVED_NAME {
            break;
        }
        clean.push(c);
    }
    let clean = clean.trim().trim_start_matches('.').to_owned();
    let clean = if clean.is_empty() { "download".to_owned() } else { clean };
    let (stem, extension) = match clean.rfind('.') {
        Some(dot) if dot > 0 => (&clean[..dot], &clean[dot..]),
        _ => (clean.as_str(), ""),
    };
    // Saves that never arrived are forgotten. Their empty reservation stays: nothing could remove it without racing a
    // finished save renamed over it, and it costs no bytes in the session's own tmpfs, which ends with the session.
    saves.retain(|_, at| at.elapsed() < SAVE_FOR);
    if saves.len() >= MAX_SAVES {
        return None;
    }
    let create = |name: &str| crate::create_new(format!("{dir}/{name}"));
    let name = (0..1000)
        .map(|n| if n == 0 { clean.clone() } else { format!("{stem} ({n}){extension}") })
        .find(|name| !saves.contains_key(name) && create(name).is_ok())?;
    saves.insert(name.clone(), std::time::Instant::now());
    Some(name)
}

#[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
impl Portal {
    /// The portal's version, a lowercase property as the portal specification names it.
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }

    async fn open_file(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
        _parent_window: &str,
        _title: &str,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let flag = |name: &str| matches!(options.get(name), Some(Value::Bool(true)));
        if flag("directory") {
            return Err(zbus::fdo::Error::NotSupported("choosing a directory".into()));
        }
        let (caller, handle, generation) = admitted(&self.shared, &header, connection, &options).await?;
        let chooser = {
            // Admitted under a Chromium that has since been relaunched: nobody would answer.
            let browser = self.shared.browser.lock().unwrap();
            if browser.generation != generation {
                return Err(refused("Chromium was relaunched"));
            }
            let mut pending = self.shared.pending.lock().unwrap();
            if pending.len() >= MAX_OPEN {
                return Err(refused("too many choosers open"));
            }
            let mut next = self.shared.next.lock().unwrap();
            *next = next.wrapping_add(1).max(1);
            pending.insert(*next, Pending { caller: caller.clone(), handle: handle.clone(), paths: Vec::new() });
            *next
        };
        let request = Request { shared: self.shared.clone(), chooser, caller };
        connection.object_server().at(handle.as_str(), request).await?;
        // Opened under the pending lock, which a relaunch takes to drop it: either the person is asked and the relaunch
        // then closes it, or it was dropped already and nobody is asked.
        let asked = {
            let pending = self.shared.pending.lock().unwrap();
            let still = pending.contains_key(&chooser);
            if still {
                self.shared.files.open_chooser(chooser, u32::from(flag("multiple")));
            }
            still
        };
        if !asked {
            let _ = connection.object_server().remove::<Request, _>(handle.as_str()).await;
            return Err(refused("Chromium was relaunched"));
        }
        let _ = self.shared.wayland.flush();
        OwnedObjectPath::try_from(handle).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    async fn save_file(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
        _parent_window: &str,
        _title: &str,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let (caller, handle, _) = admitted(&self.shared, &header, connection, &options).await?;
        let suggested = match options.get("current_name") {
            Some(Value::Str(name)) => name.to_string(),
            _ => String::new(),
        };
        let name = reserve_name(DOWNLOADS, &suggested, &mut self.shared.saves.lock().unwrap())
            .ok_or_else(|| zbus::fdo::Error::Failed("no free name".into()))?;
        let uri = file_uri(&format!("{DOWNLOADS}/{name}"));
        // Answered right away (Chromium listens on the request object before it calls): no one is asked.
        let queued = match self.shared.answers.lock().unwrap().as_ref() {
            Some(answers) => answers.try_send((caller, handle.clone(), uri)).is_ok(),
            None => false,
        };
        if !queued {
            self.shared.saves.lock().unwrap().remove(&name);
            let _ = std::fs::remove_file(format!("{DOWNLOADS}/{name}"));
            return Err(zbus::fdo::Error::LimitsExceeded("too many saves at once".into()));
        }
        OwnedObjectPath::try_from(handle).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    async fn save_files(
        &self,
        _parent_window: &str,
        _title: &str,
        _options: HashMap<&str, Value<'_>>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        Err(zbus::fdo::Error::NotSupported("saving goes through downloads".into()))
    }
}

/// The process of `pidfd` is `ancestor` or one of its descendants, and was alive throughout the check (the descriptor
/// pins its identity, so a reused number cannot pass for it).
fn descends_from(pidfd: &OwnedFd, ancestor: u32) -> bool {
    let Some(pid) = pid_of(pidfd) else { return false };
    let mut current = pid;
    let mut found = false;
    for _ in 0..32 {
        if current == ancestor {
            found = true;
            break;
        }
        match parent_of(current) {
            Some(parent) if parent > 1 => current = parent,
            _ => break,
        }
    }
    // Still the same, living process: its number was not reused while the parents were read.
    found && pid_of(pidfd) == Some(pid)
}

/// The process number a pidfd refers to, if that process still runs.
fn pid_of(pidfd: &OwnedFd) -> Option<u32> {
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd())).ok()?;
    let pid: i64 = info.lines().find_map(|l| l.strip_prefix("Pid:"))?.trim().parse().ok()?;
    u32::try_from(pid).ok().filter(|p| *p > 0)
}

fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name may contain spaces and parentheses; the fields after its last ')' are fixed.
    let after = &stat[stat.rfind(')')? + 2..];
    after.split(' ').nth(1)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_encode_everything_but_unreserved_bytes_and_slashes() {
        assert_eq!(file_uri("/uploads/1/relatório final.pdf"), "file:///uploads/1/relat%C3%B3rio%20final.pdf");
        assert_eq!(file_uri("/uploads/2/a#b?c%"), "file:///uploads/2/a%23b%3Fc%25");
    }

    #[test]
    fn saved_names_are_cleaned_bounded_and_reserved_once() {
        let scratch = crate::Scratch::new("portal");
        let dir = scratch.path();
        let mut saves = HashMap::new();
        assert_eq!(reserve_name(dir, "../relatório.pdf", &mut saves).unwrap(), "_relatório.pdf");
        // The same suggestion twice before either file exists: each save gets its own name; a file on disk counts too.
        assert_eq!(reserve_name(dir, "a.pdf", &mut saves).unwrap(), "a.pdf");
        assert_eq!(reserve_name(dir, "a.pdf", &mut saves).unwrap(), "a (1).pdf");
        std::fs::write(format!("{dir}/b.pdf"), b"").unwrap();
        assert_eq!(reserve_name(dir, "b.pdf", &mut saves).unwrap(), "b (1).pdf");
        assert_eq!(reserve_name(dir, "", &mut saves).unwrap(), "download");
        let long = format!("{}.pdf", "漢".repeat(100));
        assert!(reserve_name(dir, &long, &mut saves).unwrap().len() <= MAX_SAVED_NAME);
        while saves.len() < MAX_SAVES {
            reserve_name(dir, &format!("{}.txt", saves.len()), &mut saves).unwrap();
        }
        assert_eq!(reserve_name(dir, "one more.txt", &mut saves), None, "a new save is refused while the list is full");
    }

    #[test]
    fn a_process_descends_from_its_parents_and_not_from_strangers() {
        let me = rustix::process::pidfd_open(rustix::process::getpid(), rustix::process::PidfdFlags::empty()).unwrap();
        let parent = rustix::process::getppid().map_or(1, |p| p.as_raw_nonzero().get() as u32);
        assert!(descends_from(&me, std::process::id()));
        assert!(descends_from(&me, parent) || parent == 1);
        assert!(!descends_from(&me, u32::MAX));
    }
}
