//! The launcher's private DevTools pipe to Chromium (`--remote-debugging-pipe`: Chromium reads commands on fd 3 and
//! writes on fd 4, NUL-terminated JSON), used for three things: each download's size, so the person's own browser can
//! show its progress and time left; opening a page the display asks for (a public demo's switch, see [`Pages`]); and
//! closing Chromium for a saved profile's save ([`Pages::close_browser`]).
//!
//! The launcher attaches to every page and frame target as it appears (`Target.setAutoAttach`, flat sessions) and
//! enables only their `Page` domain, then reads `Page.downloadWillBegin` (a download's id and suggested name) and
//! `Page.downloadProgress` (its total). It never calls `setDownloadBehavior`, which would wrap Chrome's own download
//! delegate and change how downloads are written. Chromium knows a total only for a body it does not decode (net's
//! expected content size), so a known total is exactly what it writes. Everything else is skipped, messages are
//! bounded, and the pipe is never closed while Chromium runs (Chromium closes itself when its pipe goes away), so a bad
//! message only loses a size.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Largest message read; a longer one is skipped to its end.
const MAX_MESSAGE: usize = 64 * 1024;
/// Downloads remembered at once; the oldest goes first.
const MAX_DOWNLOADS: usize = 64;
/// How long a finished download is remembered, for a file announced after Chromium finished it.
const KEEP_DONE: Duration = Duration::from_secs(60);

struct Entry {
    guid: String,
    suggested: String,
    /// `None` until its first progress event; then its total, if Chromium knows one.
    total: Option<Option<u64>>,
    done: Option<Instant>,
    matched: bool,
}

/// What Chromium said about its downloads, for the Chromium running now (`generation`): a relaunched Chromium starts
/// with nothing known, and a reader of an earlier one adds nothing.
#[derive(Default)]
pub struct Sizes {
    downloads: Mutex<Vec<Entry>>,
    generation: AtomicU64,
}

/// The size of a download the launcher is about to announce.
#[derive(Debug, PartialEq)]
pub enum Lookup {
    /// Nothing certain yet: Chromium has not said enough.
    Wait,
    /// Its size, or none (unknown, or several downloads could be this one).
    Size(Option<u64>),
}

/// Whether Chromium would save a download suggested as `suggested` under `name`: the same, or with its " (n)" counter.
fn names_match(suggested: &str, name: &str) -> bool {
    if name == suggested {
        return true;
    }
    let Some(open) = name.rfind(" (") else { return false };
    let Some(close) = name[open..].find(')').map(|c| open + c) else { return false };
    let digits = &name[open + 2..close];
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && format!("{}{}", &name[..open], &name[close + 1..]) == suggested
}

impl Sizes {
    /// The size of the download saved as `name`, matched to exactly one of Chromium's downloads.
    pub fn lookup(&self, name: &str) -> Lookup {
        let mut downloads = self.downloads.lock().unwrap();
        downloads.retain(|d| d.done.is_none_or(|at| at.elapsed() < KEEP_DONE));
        // A finished download is never a live stream's: those are delivered complete, with their own size.
        let mut candidates =
            downloads.iter_mut().filter(|d| !d.matched && d.done.is_none() && names_match(&d.suggested, name));
        let Some(first) = candidates.next() else { return Lookup::Wait };
        if candidates.next().is_some() {
            return Lookup::Size(None);
        }
        match first.total {
            None => Lookup::Wait,
            Some(total) => {
                first.matched = true;
                Lookup::Size(total)
            }
        }
    }

    /// Forgets every download, for a new Chromium; returns its generation.
    pub fn renew(&self) -> u64 {
        let mut downloads = self.downloads.lock().unwrap();
        downloads.clear();
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn heard(&self, generation: u64, message: &serde_json::Value) {
        let params = &message["params"];
        let Some(guid) = params["guid"].as_str() else { return };
        let mut downloads = self.downloads.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        downloads.retain(|d| d.done.is_none_or(|at| at.elapsed() < KEEP_DONE));
        match message["method"].as_str() {
            Some("Page.downloadWillBegin") => {
                let Some(suggested) = params["suggestedFilename"].as_str() else { return };
                if downloads.iter().any(|d| d.guid == guid) {
                    return;
                }
                if downloads.len() >= MAX_DOWNLOADS {
                    downloads.remove(0);
                }
                let entry = Entry {
                    guid: guid.to_owned(),
                    suggested: suggested.to_owned(),
                    total: None,
                    done: None,
                    matched: false,
                };
                downloads.push(entry);
            }
            Some("Page.downloadProgress") => {
                let Some(d) = downloads.iter_mut().find(|d| d.guid == guid) else { return };
                if d.total.is_none() {
                    d.total = Some(params["totalBytes"].as_f64().filter(|&t| t >= 1.0).map(|t| t as u64));
                }
                if params["state"].as_str() != Some("inProgress") {
                    d.done.get_or_insert_with(Instant::now);
                }
            }
            _ => {}
        }
    }
}

/// Commands waiting for the writer.
const QUEUED: usize = 64;
/// An opening older than this is given up: never written to Chromium, and if Chromium answers it, nothing is closed.
const OPEN_WAIT: Duration = Duration::from_secs(30);

/// Pages the display asks the browser to open, in place of every page it shows: a new tab at the address, and once
/// Chromium made it (its `targetId` came back), every other page closed; if it was not made, nothing is closed. Every
/// request is answered once, through `reply`. Fenced by Chromium's generation: a relaunch answers what was pending.
pub struct Pages {
    reply: Box<dyn Fn(u32, bool) + Send + Sync>,
    inner: Mutex<Opening>,
}

#[derive(Default)]
struct Opening {
    generation: u64,
    /// The running Chromium's command queue and ids (shared with its reader).
    queue: Option<SyncSender<Command>>,
    ids: Arc<AtomicU64>,
    /// Its page targets, and the `Target.createTarget` commands awaited, with the request each answers and when.
    targets: HashSet<String>,
    pending: HashMap<u64, (u32, Instant)>,
}

impl Pages {
    pub fn new(reply: impl Fn(u32, bool) + Send + Sync + 'static) -> Arc<Pages> {
        Arc::new(Pages { reply: Box::new(reply), inner: Mutex::new(Opening::default()) })
    }

    /// A new Chromium runs: requests left for the previous one are answered as not opened.
    fn begin(&self, generation: u64, queue: SyncSender<Command>, ids: Arc<AtomicU64>) {
        let stale: Vec<u32> = {
            let mut inner = self.inner.lock().unwrap();
            let old =
                std::mem::replace(&mut *inner, Opening { generation, queue: Some(queue), ids, ..Opening::default() });
            old.pending.into_values().map(|(request, _)| request).collect()
        };
        for request in stale {
            (self.reply)(request, false);
        }
    }

    /// Opens `url` (https) for request `request`; answered once Chromium says whether it made the tab.
    pub fn open(&self, request: u32, url: &str) {
        let valid = display_protocol::frame::page_url(url);
        let queued = valid && {
            let mut inner = self.inner.lock().unwrap();
            let id = inner.ids.fetch_add(1, Ordering::Relaxed) + 1;
            let params = format!(r#"{{"url":{}}}"#, serde_json::Value::from(url));
            let bytes = frame(id, "Target.createTarget", &params, None);
            let now = Instant::now();
            let command = Command { bytes, opening: Some((id, now + OPEN_WAIT)) };
            let sent = inner.queue.as_ref().is_some_and(|queue| queue.try_send(command).is_ok());
            if sent {
                inner.pending.insert(id, (request, now));
            }
            sent
        };
        if !queued {
            (self.reply)(request, false);
        }
    }

    /// Asks the running Chromium to close the way a person quitting it does (`Browser.close`), for a saved profile's
    /// save: its windows close and its profile with them, which writes out the cookies its store still held before the
    /// browser process exits (see `profile::Closing`). Never blocks: a queue full for now is said so, to ask again.
    pub fn close_browser(&self) -> Close {
        let inner = self.inner.lock().unwrap();
        let id = inner.ids.fetch_add(1, Ordering::Relaxed) + 1;
        let bytes = frame(id, "Browser.close", "{}", None);
        match inner.queue.as_ref().map(|queue| queue.try_send(Command { bytes, opening: None })) {
            Some(Ok(())) => Close::Sent,
            Some(Err(TrySendError::Full(_))) => Close::Busy,
            Some(Err(TrySendError::Disconnected(_))) | None => Close::Gone,
        }
    }

    /// A page target of Chromium `generation` appeared or went.
    fn page(&self, generation: u64, target: &str, alive: bool) {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation != generation {
            return;
        }
        if alive {
            inner.targets.insert(target.to_owned());
        } else {
            inner.targets.remove(target);
        }
    }

    /// The answer to command `id` of Chromium `generation`, if it was a page opened: the pages to close, once it was
    /// made (the request is answered here). None for any other command.
    fn answered(&self, generation: u64, id: u64, message: &serde_json::Value) -> Option<Vec<String>> {
        let (request, made, others) = {
            let mut inner = self.inner.lock().unwrap();
            if inner.generation != generation {
                return None;
            }
            let (request, asked) = inner.pending.remove(&id)?;
            // Too late: whatever Chromium made, nobody waits for it any more, so nothing else is closed.
            let made =
                message["result"]["targetId"].as_str().map(str::to_owned).filter(|_| asked.elapsed() < OPEN_WAIT);
            let others: Vec<String> = match &made {
                Some(made) => inner.targets.iter().filter(|t| *t != made).cloned().collect(),
                None => Vec::new(),
            };
            (request, made.is_some(), others)
        };
        (self.reply)(request, made);
        Some(others)
    }

    /// Opening `id` of Chromium `generation` was given up before it was written: Chromium never answers it, so it is
    /// answered here as not opened.
    fn expired(&self, generation: u64, id: u64) {
        let request = {
            let mut inner = self.inner.lock().unwrap();
            if inner.generation != generation {
                return;
            }
            inner.pending.remove(&id)
        };
        if let Some((request, _)) = request {
            (self.reply)(request, false);
        }
    }
}

/// What became of a close asked for over the pipe ([`Pages::close_browser`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Close {
    /// Queued for Chromium.
    Sent,
    /// Its queue is full for now, behind the commands the reader sends.
    Busy,
    /// No pipe: no Chromium took one, or its writer stopped.
    Gone,
}

/// One command for the writer: its bytes, and for a page opening, its id and when it is no longer worth writing.
struct Command {
    bytes: Vec<u8>,
    opening: Option<(u64, Instant)>,
}

/// The writer end of the command queue, for [`Commands`]: each command goes whole, in order.
struct Queue(SyncSender<Command>);

impl Write for Queue {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let command = Command { bytes: buf.to_vec(), opening: None };
        self.0.send(command).map_err(|_| std::io::Error::other("the writer stopped"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Attaches to Chromium's targets on `commands` and reads their download events from `events` into `sizes`, on a
/// thread of its own that keeps both ends open as long as the launcher runs; commands go through a queue a writer
/// thread empties, so neither the reader nor whoever opens a page ever blocks on the pipe.
pub fn start(sizes: Arc<Sizes>, pages: Arc<Pages>, generation: u64, commands: OwnedFd, events: OwnedFd) {
    // Both ends stay open for the launcher's life, whatever happens to the thread: Chromium closes itself when its
    // pipe goes away.
    let (commands, events) = (std::fs::File::from(commands), std::fs::File::from(events));
    let ends = commands.try_clone().and_then(|c| events.try_clone().map(|e| (c, e)));
    std::mem::forget((commands, events));
    let Ok((commands, events)) = ends else {
        eprintln!("browser-launch: cdp: no descriptors; sizes stay unknown");
        return;
    };
    let (queue, queued) = sync_channel::<Command>(QUEUED);
    let writer = std::thread::Builder::new().name("cdp-write".into()).spawn({
        let pages = pages.clone();
        move || write(queued, commands, &pages, generation)
    });
    if writer.is_err() {
        eprintln!("browser-launch: cdp: no writer; sizes stay unknown");
        return;
    }
    let ids = Arc::new(AtomicU64::new(0));
    pages.begin(generation, queue.clone(), ids.clone());
    let spawned = std::thread::Builder::new().name("cdp".into()).spawn(move || {
        let mut commands = Commands::new(Queue(queue));
        commands.ids = ids;
        commands.pages = Some((pages, generation));
        commands.send(None, "Target.setDiscoverTargets", r#"{"discover":true}"#);
        read(events, &sizes, generation, &mut commands);
    });
    if spawned.is_err() {
        eprintln!("browser-launch: cdp: no thread; sizes stay unknown");
    }
}

/// Writes each queued command to Chromium `generation`'s pipe, in order.
fn write(queued: Receiver<Command>, mut pipe: impl Write, pages: &Pages, generation: u64) {
    for command in queued {
        // An opening that waited too long behind a stalled pipe is dropped (it was given up already) and answered.
        if let Some((id, until)) = command.opening
            && Instant::now() >= until
        {
            pages.expired(generation, id);
            continue;
        }
        if pipe.write_all(&command.bytes).is_err() {
            eprintln!("browser-launch: cdp: a command did not go");
        }
    }
}

/// A page's own frames only: attaching a worker would change its behaviour (a service worker under a debugger loses
/// its request timeouts).
const AUTO_ATTACH: &str = r#"{"autoAttach":true,"waitForDebuggerOnStart":false,"flatten":true,"filter":[{"type":"iframe"},{"exclude":true}]}"#;

/// Pages and frames followed at once.
const MAX_SESSIONS: usize = 256;

/// Commands to Chromium, each with its own id; the attachments asked for, and the sessions already set up.
struct Commands<W: Write> {
    out: W,
    ids: Arc<AtomicU64>,
    attaching: HashSet<u64>,
    sessions: HashSet<String>,
    /// Where page targets and opened pages are told, for this Chromium.
    pages: Option<(Arc<Pages>, u64)>,
}

/// A DevTools command as the pipe carries it: its JSON (`params` already JSON, `session` the target session it goes
/// to, if any), then a NUL.
fn frame(id: u64, method: &str, params: &str, session: Option<&str>) -> Vec<u8> {
    let session = session.map(|s| format!(r#","sessionId":{}"#, serde_json::Value::from(s))).unwrap_or_default();
    let mut bytes = format!(r#"{{"id":{id},"method":"{method}","params":{params}{session}}}"#).into_bytes();
    bytes.push(0);
    bytes
}

impl<W: Write> Commands<W> {
    fn new(out: W) -> Commands<W> {
        Commands { out, ids: Arc::default(), attaching: Default::default(), sessions: Default::default(), pages: None }
    }

    fn send(&mut self, session: Option<&str>, method: &str, params: &str) -> u64 {
        let id = self.ids.fetch_add(1, Ordering::Relaxed) + 1;
        if self.out.write_all(&frame(id, method, params, session)).is_err() {
            eprintln!("browser-launch: cdp: a command did not go; sizes may stay unknown");
        }
        id
    }

    fn page(&self, target: &str, alive: bool) {
        if let Some((pages, generation)) = &self.pages {
            pages.page(*generation, target, alive);
        }
    }

    /// Handles what concerns targets; false for anything else.
    fn target(&mut self, message: &serde_json::Value) -> bool {
        let params = &message["params"];
        match message["method"].as_str() {
            // A new page: attach to it.
            Some("Target.targetCreated") => {
                if params["targetInfo"]["type"] == "page"
                    && let Some(target) = params["targetInfo"]["targetId"].as_str()
                {
                    self.page(target, true);
                    if self.attaching.len() + self.sessions.len() < MAX_SESSIONS {
                        let params = format!(r#"{{"targetId":{},"flatten":true}}"#, serde_json::Value::from(target));
                        let id = self.send(None, "Target.attachToTarget", &params);
                        self.attaching.insert(id);
                    }
                }
                true
            }
            // A frame attached through its page's session; anything else, or past the bound, is let go.
            Some("Target.attachedToTarget") => {
                let Some(session) = params["sessionId"].as_str() else { return true };
                let wanted = matches!(params["targetInfo"]["type"].as_str(), Some("page" | "iframe"));
                if !(wanted && self.attached(session)) {
                    let detach = format!(r#"{{"sessionId":{}}}"#, serde_json::Value::from(session));
                    self.send(message["sessionId"].as_str(), "Target.detachFromTarget", &detach);
                }
                true
            }
            Some("Target.targetDestroyed") => {
                if let Some(target) = params["targetId"].as_str() {
                    self.page(target, false);
                }
                true
            }
            Some("Target.detachedFromTarget") => {
                if let Some(session) = params["sessionId"].as_str() {
                    self.sessions.remove(session);
                }
                true
            }
            Some(_) => false,
            // The answer to a page opened: its other pages close; or to an attachment.
            None => {
                if let (Some((pages, generation)), Some(id)) = (&self.pages, message["id"].as_u64())
                    && let Some(others) = pages.answered(*generation, id, message)
                {
                    for target in others {
                        let params = format!(r#"{{"targetId":{}}}"#, serde_json::Value::from(target));
                        self.send(None, "Target.closeTarget", &params);
                    }
                    return true;
                }
                let answered = message["id"].as_u64().is_some_and(|id| self.attaching.remove(&id));
                if answered
                    && let Some(session) = message["result"]["sessionId"].as_str()
                    && !self.attached(session)
                {
                    let detach = format!(r#"{{"sessionId":{}}}"#, serde_json::Value::from(session));
                    self.send(None, "Target.detachFromTarget", &detach);
                }
                answered
            }
        }
    }

    /// A page or frame session: its downloads are heard, and its own frames are attached in turn. False past the bound.
    fn attached(&mut self, session: &str) -> bool {
        if self.sessions.contains(session) {
            return true;
        }
        if self.sessions.len() >= MAX_SESSIONS {
            return false;
        }
        self.sessions.insert(session.to_owned());
        self.send(Some(session), "Page.enable", "{}");
        self.send(Some(session), "Target.setAutoAttach", AUTO_ATTACH);
        true
    }
}

/// Reads NUL-terminated messages until the pipe ends.
fn read<W: Write>(mut events: impl Read, sizes: &Sizes, generation: u64, commands: &mut Commands<W>) {
    let mut message = Vec::new();
    let mut skipping = false;
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let n = match events.read(&mut buffer) {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        };
        for &b in &buffer[..n] {
            if b == 0 {
                if !skipping
                    && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&message)
                    && !commands.target(&value)
                {
                    sizes.heard(generation, &value);
                }
                message.clear();
                skipping = false;
            } else if skipping || message.len() >= MAX_MESSAGE {
                skipping = true;
                message.clear();
            } else {
                message.push(b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_is_matched_by_its_name_or_chromiums_counter_only() {
        assert!(names_match("big.bin", "big.bin"));
        assert!(names_match("big.bin", "big (2).bin"));
        assert!(names_match("archive.tar.gz", "archive (1).tar.gz"));
        assert!(names_match("notes", "notes (12)"));
        assert!(!names_match("big.bin", "big (x).bin"));
        assert!(!names_match("big.bin", "other.bin"));
        assert!(!names_match("big.bin", "big ().bin"));
    }

    fn events(text: &str) -> Sizes {
        let sizes = Sizes::default();
        read(text.replace('|', "\0").as_bytes(), &sizes, 0, &mut Commands::new(Vec::new()));
        sizes
    }

    #[test]
    fn a_page_opens_and_only_then_the_others_close() {
        let answers = Arc::new(Mutex::new(Vec::new()));
        let heard = answers.clone();
        let pages = Pages::new(move |request, opened| heard.lock().unwrap().push((request, opened)));
        pages.open(1, "https://sound.play/");
        assert_eq!(*answers.lock().unwrap(), vec![(1, false)], "no Chromium yet: not opened");
        let (queue, queued) = sync_channel::<Command>(8);
        let ids = Arc::new(AtomicU64::new(0));
        pages.begin(1, queue.clone(), ids.clone());
        let mut commands = Commands::new(Queue(queue));
        commands.ids = ids;
        commands.pages = Some((pages.clone(), 1));
        let text = r#"{"method":"Target.targetCreated","params":{"targetInfo":{"type":"page","targetId":"OLD"}}}|"#;
        read(text.replace('|', "\0").as_bytes(), &Sizes::default(), 1, &mut commands);
        pages.open(2, "http://sound.play/");
        pages.open(3, "https://sound.play/");
        let sent = commands_in(&queued);
        let create = sent.iter().find(|c| c.contains("Target.createTarget")).expect("asked for the tab");
        assert!(create.contains(r#""url":"https://sound.play/""#));
        let id = id_of(create);
        // Made: the other page closes, the new one stays.
        let text = format!(r#"{{"id":{id},"result":{{"targetId":"NEW"}}}}|"#);
        read(text.replace('|', "\0").as_bytes(), &Sizes::default(), 1, &mut commands);
        let sent = commands_in(&queued);
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(sent[0].contains("Target.closeTarget") && sent[0].contains("OLD"));
        // Not made: nothing closes.
        pages.open(4, "https://typing.play/");
        let id = ids_of(&queued)[0];
        let text = format!(r#"{{"id":{id},"error":{{"message":"no"}}}}|"#);
        read(text.replace('|', "\0").as_bytes(), &Sizes::default(), 1, &mut commands);
        assert!(queued.try_iter().next().is_none());
        // A relaunch answers what was pending for the previous Chromium.
        pages.open(5, "https://typing.play/");
        let (queue, _queued) = sync_channel::<Command>(8);
        pages.begin(2, queue, Arc::default());
        assert_eq!(*answers.lock().unwrap(), vec![(1, false), (2, false), (3, true), (4, false), (5, false)]);
    }

    #[test]
    fn an_opening_given_up_before_it_was_written_is_answered_and_forgotten() {
        let answers = Arc::new(Mutex::new(Vec::new()));
        let heard = answers.clone();
        let pages = Pages::new(move |request, opened| heard.lock().unwrap().push((request, opened)));
        let (queue, queued) = sync_channel::<Command>(8);
        pages.begin(1, queue, Arc::default());
        pages.open(7, "https://sound.play/");
        pages.open(8, "https://typing.play/");
        let mut commands: Vec<Command> = queued.try_iter().collect();
        let (late, _) = commands[0].opening.unwrap();
        commands[0].opening = Some((late, Instant::now()));
        let (queue, queued) = sync_channel::<Command>(8);
        for command in commands {
            queue.send(command).unwrap();
        }
        drop(queue);
        let mut pipe = Vec::new();
        write(queued, &mut pipe, &pages, 1);
        let written = String::from_utf8(pipe).unwrap();
        assert!(!written.contains("sound.play") && written.contains("typing.play"), "{written}");
        assert_eq!(*answers.lock().unwrap(), vec![(7, false)]);
        assert!(!pages.inner.lock().unwrap().pending.contains_key(&late), "nothing left to answer it again");
        // A relaunch answers only what was still pending.
        let (queue, _queued) = sync_channel::<Command>(8);
        pages.begin(2, queue, Arc::default());
        assert_eq!(*answers.lock().unwrap(), vec![(7, false), (8, false)]);
    }

    #[test]
    fn chromium_is_closed_over_its_pipe_only_while_it_has_one() {
        let pages = Pages::new(|_, _| {});
        assert_eq!(pages.close_browser(), Close::Gone, "no Chromium yet");
        let (queue, queued) = sync_channel::<Command>(1);
        pages.begin(1, queue, Arc::default());
        assert_eq!(pages.close_browser(), Close::Sent);
        assert_eq!(pages.close_browser(), Close::Busy, "a full queue, for now");
        let sent = commands_in(&queued);
        assert_eq!(sent, [r#"{"id":1,"method":"Browser.close","params":{}}"#.to_owned() + "\0"]);
        drop(queued);
        assert_eq!(pages.close_browser(), Close::Gone, "its writer stopped");
    }

    /// The commands queued so far, as text.
    fn commands_in(queued: &Receiver<Command>) -> Vec<String> {
        queued.try_iter().map(|c| String::from_utf8(c.bytes).unwrap()).collect()
    }

    /// A command's id: every command starts `{"id":N,`.
    fn id_of(command: &str) -> u64 {
        command[6..command.find(',').unwrap()].parse().unwrap()
    }

    fn ids_of(queued: &Receiver<Command>) -> Vec<u64> {
        commands_in(queued).iter().map(|c| id_of(c)).collect()
    }

    #[test]
    fn every_page_and_frame_is_attached_and_heard() {
        let attached = |s: &str, t: &str| {
            format!(
                r#"{{"method":"Target.attachedToTarget","params":{{"sessionId":"{s}","targetInfo":{{"type":"{t}"}}}}}}|"#
            )
        };
        let created = r#"{"method":"Target.targetCreated","params":{"targetInfo":{"type":"page","targetId":"T1"}}}|"#;
        let worker = r#"{"method":"Target.targetCreated","params":{"targetInfo":{"type":"worker","targetId":"T2"}}}|"#;
        let answer = r#"{"id":1,"result":{"sessionId":"S9"}}|"#;
        let mut commands = Commands::new(Vec::new());
        let text = created.to_owned()
            + worker
            + answer
            + &attached("S1", "page")
            + &attached("S2", "service_worker")
            + &attached("S3", "iframe")
            + &attached("S9", "page");
        read(text.replace('|', "\0").as_bytes(), &Sizes::default(), 0, &mut commands);
        let sent: Vec<String> =
            String::from_utf8(commands.out).unwrap().split('\0').filter(|c| !c.is_empty()).map(str::to_owned).collect();
        assert_eq!(sent.len(), 8, "{sent:?}");
        assert!(sent[0].contains(r#""method":"Target.attachToTarget""#) && sent[0].contains(r#""targetId":"T1""#));
        assert!(sent[1].contains(r#""method":"Page.enable""#) && sent[1].contains(r#""sessionId":"S9""#));
        assert!(
            sent[2].contains("Target.setAutoAttach") && sent[2].contains(r#"[{"type":"iframe"},{"exclude":true}]"#)
        );
        assert!(sent[3].contains("Page.enable") && sent[3].contains(r#""sessionId":"S1""#));
        assert!(
            sent[5].contains("Target.detachFromTarget") && sent[5].contains(r#"{"sessionId":"S2"}"#),
            "a worker is let go"
        );
        assert!(sent[6].contains("Page.enable") && sent[6].contains(r#""sessionId":"S3""#));
        // A detached session leaves room, and attaching it again sets it up again.
        let mut commands = Commands::new(Vec::new());
        let detached = r#"{"method":"Target.detachedFromTarget","params":{"sessionId":"S1"}}|"#;
        let text = attached("S1", "page") + detached + &attached("S1", "page");
        read(text.replace('|', "\0").as_bytes(), &Sizes::default(), 0, &mut commands);
        assert_eq!(String::from_utf8(commands.out).unwrap().matches("Page.enable").count(), 2);
        assert!(sent.iter().all(|c| !c.contains("setDownloadBehavior")));
    }

    #[test]
    fn a_size_is_given_once_after_progress_and_never_when_ambiguous() {
        let begin = |g: &str, n: &str| {
            format!(r#"{{"method":"Page.downloadWillBegin","params":{{"guid":"{g}","suggestedFilename":"{n}"}}}}|"#)
        };
        let progress = |g: &str, t: u64| {
            format!(
                r#"{{"method":"Page.downloadProgress","params":{{"guid":"{g}","totalBytes":{t},"state":"inProgress"}}}}|"#
            )
        };
        let sizes = events(&(begin("a", "big.bin") + "{not json}|" + &"x".repeat(MAX_MESSAGE + 1) + "|"));
        assert_eq!(sizes.lookup("big.bin"), Lookup::Wait, "no progress yet");
        let sizes = events(&(begin("a", "big.bin") + &progress("a", 4_831_838_208)));
        assert_eq!(sizes.lookup("big.bin"), Lookup::Size(Some(4_831_838_208)));
        assert_eq!(sizes.lookup("big.bin"), Lookup::Wait, "matched once");
        let sizes = events(&(begin("a", "big.bin") + &progress("a", 0)));
        assert_eq!(sizes.lookup("big (1).bin"), Lookup::Size(None), "a total Chromium does not know");
        let sizes = events(&(begin("a", "big.bin") + &begin("b", "big.bin") + &progress("a", 5) + &progress("b", 6)));
        assert_eq!(sizes.lookup("big.bin"), Lookup::Size(None), "two could be this one");
        assert_eq!(events("").lookup("big.bin"), Lookup::Wait);
        let done = |g: &str| {
            format!(
                r#"{{"method":"Page.downloadProgress","params":{{"guid":"{g}","totalBytes":5,"state":"completed"}}}}|"#
            )
        };
        let sizes = events(&(begin("a", "big.bin") + &done("a") + &begin("b", "big.bin") + &progress("b", 6)));
        assert_eq!(sizes.lookup("big.bin"), Lookup::Size(Some(6)), "a finished download is never a live stream's");
    }

    #[test]
    fn a_relaunched_chromium_starts_with_nothing_known_and_an_old_reader_adds_nothing() {
        let text = concat!(
            r#"{"method":"Page.downloadWillBegin","params":{"guid":"a","suggestedFilename":"big.bin"}}|"#,
            r#"{"method":"Page.downloadProgress","params":{"guid":"a","totalBytes":7,"state":"inProgress"}}|"#,
        )
        .replace('|', "\0");
        let sizes = Sizes::default();
        read(text.as_bytes(), &sizes, 0, &mut Commands::new(Vec::new()));
        let generation = sizes.renew();
        assert_eq!(sizes.lookup("big.bin"), Lookup::Wait, "forgotten");
        read(text.as_bytes(), &sizes, 0, &mut Commands::new(Vec::new()));
        assert_eq!(sizes.lookup("big.bin"), Lookup::Wait, "the earlier Chromium's reader is ignored");
        read(text.as_bytes(), &sizes, generation, &mut Commands::new(Vec::new()));
        assert_eq!(sizes.lookup("big.bin"), Lookup::Size(Some(7)));
    }
}
