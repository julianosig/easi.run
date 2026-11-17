//! The launcher's private DevTools pipe to Chromium (`--remote-debugging-pipe`: Chromium reads commands on fd 3 and
//! writes on fd 4, NUL-terminated JSON), used for six things: each download's size, so the person's own browser can
//! show its progress and time left; opening a page the display asks for (a public demo's switch, see [`Pages`]);
//! closing Chromium for a saved profile's save ([`Pages::close_browser`]); reading a page's structure for the agent
//! ([`Pages::inspect`]), in an isolated world, which needs no `Runtime.enable`, the debugger's mark pages look for;
//! hearing that a tab's page crashed, and why ([`Crashes`]); and saying once Chromium's pages have run what they had
//! to run before, an agent's keys among it ([`Pages::settle`]).
//!
//! The launcher attaches to every page and frame target as it appears (`Target.setAutoAttach`, flat sessions) and
//! enables only their `Page` domain, then reads `Page.downloadWillBegin` (a download's id and suggested name) and
//! `Page.downloadProgress` (its total). It never calls `setDownloadBehavior`, which would wrap Chrome's own download
//! delegate and change how downloads are written. Chromium knows a total only for a body it does not decode (net's
//! expected content size), so a known total is exactly what it writes. Everything else is skipped, messages are
//! bounded, and the pipe is never closed while Chromium runs (Chromium closes itself when its pipe goes away), so a bad
//! message only loses a size or answers a tab it was opening as failed.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use display_protocol::frame::PAGE_OPEN_WAIT;

/// Largest message read; a longer one is skipped to its end: past a page's largest answer ([`INSPECT_SCRIPT`]) as JSON
/// inside JSON, each byte escaped at worst.
const MAX_MESSAGE: usize = 1024 * 1024;
/// What reads a page for [`Pages::inspect`]: a function of the question.
const INSPECT_SCRIPT: &str = include_str!("inspect.js");
/// What [`Pages::settle`] asks of each page and frame: to evaluate nothing, which its main thread does in turn behind
/// what it had to run before, the keys sent to it among it. It needs no `Runtime.enable`, changes nothing a page can
/// read, and, unlike a key event sent over this pipe (which would follow the keys exactly), never gives a page the
/// focus the browser took from it.
const MARK: &str = r#"{"expression":"0"}"#;
/// What a strict settle asks first, in a world of its own in each page (as [`Pages::inspect`] reads one, so the page's
/// own scripts do not stand in for what it calls; it takes one of the document's frame request numbers, as an
/// extension's script does): a page that shows answers once it has drawn a frame. A page that has just started is
/// often still giving its main thread the keys when it is asked plainly, and has given them by the time it draws
/// (measured: the address typed after a shortcut was lost in 1 of 150 fresh sessions asked plainly, in none of 150
/// asked this way); nothing orders the two by construction. A page that does not show draws none and answers at once.
const FRAME: &str = "document.visibilityState==='visible'?new Promise(done=>requestAnimationFrame(()=>done(0))):0";
/// How many times in a row each page is asked: the answer to a second asking leaves only after the page has long sent
/// the answers to the keys it handled before the first, so the browser has them by then.
const MARKS: u8 = 2;
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
    claim: Claim,
}

/// Whether a file was announced that could be this download.
#[derive(Clone, Copy, PartialEq)]
enum Claim {
    /// None yet.
    Open,
    /// One, which could be no other download: its file.
    Taken,
    /// One that could also be another: either may still have a file to come.
    Shared,
}

/// What Chromium said about its downloads, for the Chromium running now: a relaunched Chromium starts with nothing
/// known, and a reader of an earlier one adds nothing.
#[derive(Default)]
pub struct Sizes {
    downloads: Mutex<Downloads>,
}

#[derive(Default)]
struct Downloads {
    /// The Chromium they are of, renewed with them under one lock.
    generation: u64,
    entries: Vec<Entry>,
    /// A download not taken was forgotten (to make room, or finished long ago): its file, if still to come, could
    /// match another one alone, so no file is given a total until Chromium relaunches.
    lost: bool,
}

impl Downloads {
    /// Forgets the finished downloads kept long enough, and the oldest one too when `room` is wanted for another.
    fn forget(&mut self, room: bool) {
        let lost = &mut self.lost;
        self.entries.retain(|d| {
            let keep = d.done.is_none_or(|at| at.elapsed() < KEEP_DONE);
            *lost |= !keep && d.claim != Claim::Taken;
            keep
        });
        if room && self.entries.len() >= MAX_DOWNLOADS {
            self.lost |= self.entries.remove(0).claim != Claim::Taken;
        }
    }
}

/// The size of a download the launcher is about to announce.
#[derive(Debug, PartialEq)]
pub enum Lookup {
    /// Nothing certain yet: Chromium has not said enough.
    Wait,
    /// Its size, or none (unknown, or not certainly this download's).
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
    /// The size of the download saved as `name`, by one rule: a file is given a total only when exactly one download
    /// not taken could be it, still running with its total said, and no download not taken was forgotten since
    /// Chromium launched; otherwise its size is unknown. A finished one gives none: the file it was could still be
    /// announced before its completion is seen. `last`: the file is announced now whatever is known (never `Wait`), so
    /// it takes what it could be all the same: a download no file took is one whose file may still come.
    pub fn lookup(&self, name: &str, last: bool) -> Lookup {
        let mut downloads = self.downloads.lock().unwrap();
        downloads.forget(false);
        if downloads.lost {
            return Lookup::Size(None);
        }
        let mut candidates: Vec<&mut Entry> = downloads
            .entries
            .iter_mut()
            .filter(|d| d.claim != Claim::Taken && names_match(&d.suggested, name))
            .collect();
        match &mut candidates[..] {
            [] if last => Lookup::Size(None),
            [] => Lookup::Wait,
            [only] if only.claim == Claim::Open => {
                let total = match (only.done, only.total) {
                    (None, Some(total)) => total,
                    _ if last => None,
                    // Its total not said yet, or finished: the file's own completion gives its size.
                    _ => return Lookup::Wait,
                };
                only.claim = Claim::Taken;
                Lookup::Size(total)
            }
            // Several could be this one: each may still have a file to come, which could match a later one alone.
            several => {
                for d in several {
                    d.claim = Claim::Shared;
                }
                Lookup::Size(None)
            }
        }
    }

    /// Forgets every download, for a new Chromium; returns its generation.
    pub fn renew(&self) -> u64 {
        let mut downloads = self.downloads.lock().unwrap();
        let generation = downloads.generation + 1;
        *downloads = Downloads { generation, ..Downloads::default() };
        generation
    }

    fn heard(&self, generation: u64, message: &serde_json::Value) {
        let params = &message["params"];
        let Some(guid) = params["guid"].as_str() else { return };
        let mut downloads = self.downloads.lock().unwrap();
        if downloads.generation != generation {
            return;
        }
        match message["method"].as_str() {
            Some("Page.downloadWillBegin") => {
                let Some(suggested) = params["suggestedFilename"].as_str() else { return };
                if downloads.entries.iter().any(|d| d.guid == guid) {
                    return;
                }
                downloads.forget(true);
                let entry = Entry {
                    guid: guid.to_owned(),
                    suggested: suggested.to_owned(),
                    total: None,
                    done: None,
                    claim: Claim::Open,
                };
                downloads.entries.push(entry);
            }
            Some("Page.downloadProgress") => {
                let Some(d) = downloads.entries.iter_mut().find(|d| d.guid == guid) else { return };
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

/// Pages the display asks the browser to open, in place of every page it shows: a new tab at the address, and once
/// Chromium made it (its `targetId` came back), every other page closed; if it was not made, nothing is closed. The
/// latest opening made wins: an older one Chromium makes after it, or one made too late, closes only its own tab. Every
/// request is answered once, through `reply`. Fenced by Chromium's generation: a relaunch answers what was pending.
pub struct Pages {
    reply: Box<dyn Fn(u32, bool) + Send + Sync>,
    /// The tabs a session may keep (`BROWSER_MAX_TABS`), if any, and where a new one closed past them is told.
    limit: Option<u16>,
    refused: Box<dyn Fn(u16) + Send + Sync>,
    crashes: Crashes,
    /// Whether a debugger may be attached to the pages (a session with developer tools): a strict settle then asks
    /// them for no frame (see [`Pages::debugged`]).
    debugged: std::sync::atomic::AtomicBool,
    inner: Mutex<Opening>,
}

/// Where a tab's crash is told, with its cause: whether the kernel ended the page's process as the browser reached its
/// memory limit. Chromium says only that the process was killed (`Target.targetCrashed`, error code 9, SIGKILL's); the
/// browser's own count of such kills (`oom_kill` in its cgroup's `memory.events`) says by whom: one that grew since it
/// was last read is the kernel's. A kill of another of the browser's processes (its GPU process, which Chromium starts
/// again) grows it too, and is taken for the next page killed.
pub struct Crashes {
    told: Box<dyn Fn(bool) + Send + Sync>,
    kills: Box<dyn Fn() -> Option<u64> + Send + Sync>,
    /// The count as last read; none while it could not be read.
    seen: Mutex<Option<u64>>,
}

impl Crashes {
    /// `told` hears each crash and whether memory was its cause; `kills` reads the count now.
    pub fn new(
        told: impl Fn(bool) + Send + Sync + 'static,
        kills: impl Fn() -> Option<u64> + Send + Sync + 'static,
    ) -> Crashes {
        let seen = Mutex::new(kills());
        Crashes { told: Box::new(told), kills: Box::new(kills), seen }
    }

    /// Whether the kernel's count grew since it was last read, which it is read as now: one reading at a time, so a
    /// later one never puts an earlier count back.
    fn grew(&self) -> bool {
        let mut seen = self.seen.lock().unwrap();
        let now = (self.kills)();
        let grew = matches!((now, *seen), (Some(now), Some(before)) if now > before);
        if now.is_some() {
            *seen = now;
        }
        grew
    }
}

/// The signal the kernel ends a process with for its cgroup's memory limit, as Chromium's error code says it.
const SIGKILL: i64 = 9;

/// The count of processes the kernel ended for the memory limit, in a cgroup's `memory.events`.
pub fn oom_kills(events: &str) -> Option<u64> {
    events.lines().find_map(|line| line.strip_prefix("oom_kill ")?.trim().parse().ok())
}

/// How long a Chromium that starts has to restore its tabs (a saved session's, all it had open) before the tab limit
/// holds: what it restores is the session's own, never refused.
const RESTORE_GRACE: Duration = Duration::from_secs(20);

#[derive(Default)]
struct Opening {
    generation: u64,
    /// When this Chromium started (see [`RESTORE_GRACE`]).
    began: Option<Instant>,
    /// The running Chromium's command queue and ids (shared with its reader).
    queue: Option<SyncSender<Command>>,
    ids: Arc<AtomicU64>,
    /// Its page targets, and the `Target.createTarget` commands awaited, with the request each answers and when.
    targets: HashSet<String>,
    pending: HashMap<u64, (u32, Instant)>,
    /// The command of the latest opening made: the page shown.
    shown: u64,
    /// Its page sessions, and the page reading under way: one at a time, a new one in its place.
    sessions: HashSet<String>,
    inspection: Option<Inspection>,
    /// The sessions of its pages' frames that run in processes of their own.
    frames: HashSet<String>,
    /// The settles under way, oldest first.
    fences: Vec<Fence>,
    /// The pages whose lifecycle events a navigation turned on and could not turn off (the queue was full): turned
    /// off when the pages are next read.
    loud: Arc<Mutex<Vec<String>>>,
}

impl Opening {
    /// The reading under way, once it waits for no page: with a script that no page has run yet, the script goes to
    /// the page that says it shows (to none when none says so: a script never runs on a page nobody sees), and has
    /// its own time from there; otherwise the reading ends, with the answer it has.
    fn inspected(&mut self) {
        // One nobody waits for any more is not gone on with: a script given up for time never starts late. A
        // navigation already given up, which waits for the browser's word alone, ends below once it has it.
        let stopping = |inspection: &Inspection| inspection.navigation.as_ref().is_some_and(|n| n.stopping);
        let late = |inspection: &Inspection| inspection.began.elapsed() >= inspection.wait && !stopping(inspection);
        if self.inspection.as_ref().is_some_and(late) {
            self.give_up("timeout");
            return;
        }
        let Opening { inspection: slot, queue, ids, loud, .. } = self;
        let Some(inspection) = slot.as_mut().filter(|inspection| inspection.waiting.is_empty()) else { return };
        if let Some(navigation) = inspection.navigation.as_mut() {
            // The page it navigated loads: its lifecycle says when (see [`Pages::lifecycle`]).
            if navigation.loading() {
                return;
            }
            // The page that shows is found (or none says it shows): its lifecycle events are turned on first, so
            // the load of the document the navigation begins is told apart from any other's.
            if navigation.page.is_none() && navigation.outcome.is_none() {
                // One page, and no doubt which: with several windows showing one each and the keyboard in none of
                // them, or a page that did not say whether it shows, none is taken anywhere.
                let shown = sole_visible(&inspection.answers, inspection.missed)
                    .map(|shown| shown.map(|(shown, _)| shown.clone()));
                inspection.answers.clear();
                let asked = shown.clone().ok().flatten().zip(queue.as_ref()).and_then(|(shown, queue)| {
                    let id = ids.fetch_add(1, Ordering::Relaxed) + 1;
                    let bytes = frame(id, "Page.setLifecycleEventsEnabled", r#"{"enabled":true}"#, Some(&shown));
                    queue.try_send(Command { bytes, opening: None, script: None }).ok().map(|()| (id, shown))
                });
                match (asked, shown) {
                    (Some((id, page)), _) => {
                        inspection.waiting.insert(id, (page.clone(), Step::Lifecycle));
                        // Its main frame, and the document there now: what the page says of any other document
                        // of that frame from here on is of the navigation, or of what the page did after it.
                        if let Some((frame, document)) = inspection.frames.get(&page) {
                            (navigation.frame, navigation.before) = (Some(frame.clone()), Some(document.clone()));
                        }
                        navigation.page = Some(page);
                        navigation.quiet =
                            queue.clone().map(|queue| Quiet { queue, ids: ids.clone(), loud: loud.clone() });
                        return;
                    }
                    (None, Err(())) => navigation.outcome = Some("ambiguous"),
                    (None, Ok(_)) => navigation.outcome = Some("no_page"),
                }
            }
        }
        if let Some(script) = &inspection.script
            && let Some((shown, _)) = visible_answer(&inspection.answers).cloned()
            && let Some(queue) = queue.as_ref()
        {
            // The browser itself ends a script that runs on without yielding (a loop that does not end) when its time
            // is over: it would keep its page from answering anyone, its person and every later reading of any page
            // (a reading waits for each). Only that evaluation's own run is ended, never what else the page executes;
            // one that waits (for a promise) is not running, and is given up as before.
            let time = display_protocol::frame::SCRIPT_RUN.as_millis() as u64;
            let params = serde_json::json!({
                "expression": script, "userGesture": true, "awaitPromise": true, "timeout": time,
            });
            let id = ids.fetch_add(1, Ordering::Relaxed) + 1;
            let bytes = frame(id, "Runtime.evaluate", &params.to_string(), Some(&shown));
            // What the pages said of themselves is used once: a script is sent to one page, one time.
            inspection.answers.clear();
            if queue.try_send(Command { bytes, opening: None, script: inspection.hold.clone() }).is_ok() {
                inspection.waiting.insert(id, (shown, Step::Script));
                (inspection.began, inspection.wait) = (Instant::now(), display_protocol::frame::SCRIPT_RUN);
                return;
            }
        }
        if let Some(done) = slot.take() {
            done.finish();
        }
    }

    /// The reading under way is waited for no more (its time is over, or it was given up): a reading or a script
    /// ends with no answer, as its going does; a navigation answers `outcome` (or what already became of it) and how
    /// far it got. Unless the browser has the navigation's command and has not said what it made of it: then the
    /// browser is told to stop loading, and only its answers end the navigation (see [`Opening::stopped`]), so
    /// whoever reads the navigation's answer knows nothing of it begins after; a browser that never answers leaves
    /// it unanswered.
    fn give_up(&mut self, outcome: &'static str) {
        let Opening { inspection: slot, queue, ids, .. } = self;
        let Some(inspection) = slot.as_mut() else { return };
        let Some(navigation) = inspection.navigation.as_mut() else {
            *slot = None;
            return;
        };
        if navigation.stopping {
            return;
        }
        navigation.outcome.get_or_insert(outcome);
        let written = inspection.hold.as_ref().is_some_and(give_up);
        let unanswered = inspection.waiting.values().any(|(_, step)| *step == Step::Navigate);
        inspection.waiting.retain(|_, (_, step)| *step == Step::Navigate);
        inspection.answers.clear();
        if written && unanswered {
            navigation.stopping = true;
            // Written under a hold of its own: once the navigation is answered for, a stop still waiting is never
            // written, and one the browser has is waited for. It then stops nothing a person began after.
            if let (Some(page), Some(queue)) = (&navigation.page, queue.as_ref()) {
                let (id, stop) = (ids.fetch_add(1, Ordering::Relaxed) + 1, Hold::default());
                let bytes = frame(id, "Page.stopLoading", "{}", Some(page));
                if queue.try_send(Command { bytes, opening: None, script: Some(stop.clone()) }).is_ok() {
                    inspection.waiting.insert(id, (page.clone(), Step::Stop));
                    navigation.stop = Some(stop);
                }
            }
            return;
        }
        if let Some(mut over) = slot.take() {
            over.waiting.clear();
            over.finish();
        }
    }

    /// A navigation given up while the browser had its command: once the browser answered that, its command to
    /// stop loading is given up if it still waits to be written, and waited for if the browser has it.
    fn stopped(&mut self) {
        let Some(inspection) = self.inspection.as_mut() else { return };
        let Some(navigation) = inspection.navigation.as_ref().filter(|navigation| navigation.stopping) else { return };
        if inspection.waiting.values().any(|(_, step)| *step == Step::Navigate) {
            return;
        }
        if navigation.stop.as_ref().is_some_and(|stop| !give_up(stop)) {
            inspection.waiting.retain(|_, (_, step)| *step != Step::Stop);
        }
    }

    /// The page a navigation took elsewhere went on to another document by itself: what was read of the one
    /// before is dropped, and the new one's load is waited for, within what is left of the navigation's time to
    /// load.
    fn moved(&mut self) {
        let Some(inspection) = self.inspection.as_mut() else { return };
        let Some(navigation) = inspection.navigation.as_ref() else { return };
        inspection.waiting.clear();
        inspection.answers.clear();
        let now = Instant::now();
        (inspection.began, inspection.wait) = (now, navigation.until.saturating_duration_since(now));
    }

    /// What a navigation's page said of a document of its main frame made this of the navigation.
    fn heard(&mut self, session: &str, heard: Heard) {
        match heard {
            Heard::Nothing => {
                // It only tells the time: a navigation past its own is answered as it stands.
                if self.inspection.as_ref().is_some_and(|inspection| inspection.began.elapsed() >= inspection.wait) {
                    self.give_up("timeout");
                }
            }
            Heard::Loaded => {
                self.arrived(session);
                self.inspected();
            }
            Heard::Moved { loaded } => {
                self.moved();
                if loaded {
                    self.arrived(session);
                }
                self.inspected();
            }
            Heard::Lost => self.give_up("timeout"),
        }
    }

    /// The navigation under way has its page where it goes (it loaded, or stays the document it was): the page is
    /// read there when the navigation asks for it, on `session` alone.
    fn arrived(&mut self, session: &str) {
        let Opening { inspection, queue, ids, .. } = self;
        let Some(inspection) = inspection.as_mut() else { return };
        let Some(navigation) = inspection.navigation.as_ref() else { return };
        if !matches!(navigation.outcome, Some("loaded" | "same_document")) {
            return;
        }
        let (Some(read), Some(queue)) = (navigation.read.clone(), queue.as_ref()) else { return };
        inspection.query = read;
        let id = ids.fetch_add(1, Ordering::Relaxed) + 1;
        let bytes = frame(id, "Page.getFrameTree", "{}", Some(session));
        if queue.try_send(Command { bytes, opening: None, script: None }).is_ok() {
            inspection.waiting.insert(id, (session.to_owned(), Step::Frame));
        }
        // The reading has a reading's own time from here, however long the page took to load.
        (inspection.began, inspection.wait) = (Instant::now(), display_protocol::frame::INSPECT_WAIT);
    }

    /// Asks every page and frame that owes no settle an answer to evaluate [`MARK`]; `strict`, a page is asked for its
    /// frame first instead, in whose world it is then asked [`FRAME`] (see [`Opening::step`]). A frame of a page is
    /// asked plainly either way: the browser draws none for one out of sight, which says it shows all the same. The
    /// commands sent, each with its session and step.
    fn mark(&mut self, strict: bool) -> HashMap<u64, (String, Step)> {
        let owing: HashSet<&String> = self.fences.iter().flat_map(|f| f.waiting.values().map(|(s, _)| s)).collect();
        let mut waiting = HashMap::new();
        let Some(queue) = &self.queue else { return waiting };
        let pages = self.sessions.iter().map(|session| (session, strict));
        for (session, strict) in pages.chain(self.frames.iter().map(|session| (session, false))) {
            if owing.contains(session) {
                continue;
            }
            let (method, params, step) = match strict {
                true => ("Page.getFrameTree", "{}", Step::Frame),
                false => ("Runtime.evaluate", MARK, Step::Read),
            };
            let id = self.ids.fetch_add(1, Ordering::Relaxed) + 1;
            let bytes = frame(id, method, params, Some(session));
            if queue.try_send(Command { bytes, opening: None, script: None }).is_ok() {
                waiting.insert(id, (session.clone(), step));
            }
        }
        waiting
    }

    /// What follows the answer `message` to a step of a strict asking, for `session`: its frame's world, then
    /// [`FRAME`] in that world. Its id and step, or none when that was the last, or the page gave nothing to go on
    /// with (a page that went meanwhile, a full queue): it is then waited for no more.
    fn step(&mut self, session: &str, step: Step, message: &serde_json::Value) -> Option<(u64, Step)> {
        let result = &message["result"];
        let (method, params, next) = match step {
            Step::Frame => {
                let frame_id = result["frameTree"]["frame"]["id"].as_str()?;
                let params = serde_json::json!({"frameId": frame_id, "worldName": ""});
                ("Page.createIsolatedWorld", params, Step::World)
            }
            Step::World => {
                let context = result["executionContextId"].as_u64()?;
                let params = serde_json::json!({"expression": FRAME, "contextId": context, "awaitPromise": true});
                ("Runtime.evaluate", params, Step::Read)
            }
            // A settle evaluates no script of an agent's.
            Step::Read | Step::Script | Step::Lifecycle | Step::Navigate | Step::Stop => return None,
        };
        let id = self.ids.fetch_add(1, Ordering::Relaxed) + 1;
        let bytes = frame(id, method, &params.to_string(), Some(session));
        self.queue.as_ref()?.try_send(Command { bytes, opening: None, script: None }).ok().map(|()| (id, next))
    }

    /// The settle at `at` has no answer left to wait for: asked again if it is to be, else taken out, with what to
    /// call. None while it still waits.
    fn answered(&mut self, at: usize) -> Option<Box<dyn FnOnce() + Send>> {
        if !self.fences[at].waiting.is_empty() {
            return None;
        }
        if self.fences[at].more > 0 {
            let more = self.fences[at].more - 1;
            // Out while the pages are asked, so the ones it asked before owe it nothing.
            let mut fence = self.fences.remove(at);
            fence.waiting = self.mark(false);
            fence.more = more;
            if !fence.waiting.is_empty() {
                self.fences.insert(at, fence);
                return None;
            }
            return Some(fence.done);
        }
        Some(self.fences.remove(at).done)
    }
}

/// A settle under way ([`Pages::settle`]): the commands not answered yet, each with the session it went to and the
/// step it is of, how many more times the pages are asked once those are, and what to call at the end.
struct Fence {
    waiting: HashMap<u64, (String, Step)>,
    more: u8,
    done: Box<dyn FnOnce() + Send>,
}

/// A reading of the pages' structure: each page's next step, what they said (each with its session), and where the
/// answer goes. With a `script`, the reading only finds the page that shows, and the script is then evaluated there,
/// in the page's own world: its answer is the one that goes.
struct Inspection {
    began: Instant,
    /// How long it may take from `began`: a reading's bound, then a script's own once it runs.
    wait: Duration,
    query: String,
    waiting: HashMap<u64, (String, Step)>,
    answers: Vec<(String, String)>,
    /// Each page's main frame and the document in it, as the page said when asked for its frame; and whether a page
    /// asked did not answer (a step of its reading failed): what it shows is then not known.
    frames: HashMap<String, (String, String)>,
    missed: bool,
    script: Option<String>,
    /// For a navigation of the page that shows: the reading first finds that page, as for a script.
    navigation: Option<Navigation>,
    /// For one asked for a script or a navigation, begun by now or not: what its command is written under.
    hold: Option<Hold>,
    /// Where the answer goes: closed, with or without one, only once this is gone.
    answer: Option<OwnedFd>,
}

/// A navigation of the page that shows ([`Pages::navigate`]): where to, what to read there, and how far it is.
struct Navigation {
    url: String,
    /// What to read of the page once it is there, as a reading's query.
    read: Option<String>,
    /// The session of the page it navigates, once that page is found, and what turns its lifecycle events off again.
    page: Option<String>,
    quiet: Option<Quiet>,
    /// The page's main frame, and the document that was in it when the navigation was asked.
    frame: Option<String>,
    before: Option<String>,
    /// Until when its document may load, however many documents the page goes through.
    until: Instant,
    /// Whether the browser answered the navigation, and the document awaited or read since: the one the browser
    /// began for it, the one before when it stays in that, or one the page went on to by itself.
    replied: bool,
    loader: Option<String>,
    /// The documents that are no longer the page's: what the page still says of them is of no use.
    past: Vec<String>,
    /// The other documents of the main frame the page spoke of before the browser's answer, in the order they first
    /// came, each with whether it said it loaded: the navigation's own may be among them, and ones after it.
    seen: Vec<(String, bool)>,
    /// How many times the page went on to another document by itself (a redirect of its own) and was followed.
    moves: u8,
    /// Given up while the browser had the navigation and had not answered it: only its answers end it, and the
    /// command to stop loading it was sent then, under a hold of its own.
    stopping: bool,
    stop: Option<Hold>,
    /// Whether the browser began a new document for it: the page may no longer be the document it was.
    began: bool,
    /// What became of it, once that is known, and the browser's own word when it failed.
    outcome: Option<&'static str>,
    error: Option<String>,
}

/// What a navigation's page said of a document of its main frame makes of the navigation.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Heard {
    Nothing,
    /// The document awaited loaded.
    Loaded,
    /// The page went on to another document, which is awaited now, or has loaded already.
    Moved {
        loaded: bool,
    },
    /// It went on to more documents than are followed.
    Lost,
}

/// The most documents a page may go on to by itself before its navigation is given up.
const MOVES: u8 = 5;
/// The most documents kept of what a page said before the browser answered its navigation.
const SEEN: usize = 16;

impl Navigation {
    fn to(url: &str, read: Option<String>) -> Navigation {
        Navigation {
            url: url.to_owned(),
            read,
            page: None,
            quiet: None,
            frame: None,
            before: None,
            until: Instant::now() + display_protocol::frame::NAVIGATE_LOAD,
            replied: false,
            loader: None,
            past: Vec::new(),
            seen: Vec::new(),
            moves: 0,
            stopping: false,
            stop: None,
            began: false,
            outcome: None,
            error: None,
        }
    }

    /// Whether it still waits for the page it navigated to load.
    fn loading(&self) -> bool {
        self.outcome.is_none() && self.loader.is_some()
    }

    /// The page said `name` of the document `loader` of its main frame, before the browser answered the navigation:
    /// kept, to be known for what it is then.
    fn saw(&mut self, loader: &str, name: &str) {
        if self.before.as_deref() == Some(loader) {
            return;
        }
        let at = self.seen.iter().position(|(seen, _)| seen == loader).or_else(|| {
            (self.seen.len() < SEEN).then(|| self.seen.push((loader.to_owned(), false))).map(|()| self.seen.len() - 1)
        });
        if let Some(at) = at {
            self.seen[at].1 |= name == "load";
        }
    }

    /// The page said `name` of the document `loader` of its main frame, after the browser answered the navigation.
    /// A document that is neither the one awaited nor one from before is one the page went on to by itself,
    /// whatever it says of it first.
    fn heard(&mut self, loader: &str, name: &str) -> Heard {
        let goes_on = matches!(self.outcome, None | Some("loaded" | "same_document"));
        if !goes_on || self.past.iter().any(|past| past == loader) {
            return Heard::Nothing;
        }
        // It stays in a document nobody named: this is the one.
        if self.loader.is_none() {
            self.loader = Some(loader.to_owned());
        }
        if self.loader.as_deref() == Some(loader) {
            if name == "load" && self.outcome.is_none() {
                self.outcome = Some("loaded");
                return Heard::Loaded;
            }
            return Heard::Nothing;
        }
        if self.moves >= MOVES {
            self.outcome = None;
            return Heard::Lost;
        }
        self.moves += 1;
        self.past.extend(self.loader.replace(loader.to_owned()));
        let loaded = name == "load";
        (self.began, self.outcome) = (true, loaded.then_some("loaded"));
        Heard::Moved { loaded }
    }

    /// The browser answered the navigation: it stays in the document shown (no `loader`), or it began one. What
    /// the page said before that answer is read now: of the navigation's own document, and of any it went on to.
    fn replied(&mut self, frame: Option<&str>, loader: Option<&str>) -> Heard {
        self.replied = true;
        let mut seen = std::mem::take(&mut self.seen);
        // Not the frame the page was asked for: what was kept was of another's documents.
        if frame.is_some() && self.frame.as_deref() != frame {
            self.frame = frame.map(str::to_owned);
            seen.clear();
        }
        match loader {
            Some(loader) => {
                self.began = true;
                self.past.extend(self.before.clone());
                // What came before its own document's first word was of documents before it.
                let own = seen.iter().position(|(seen, _)| seen == loader).unwrap_or(seen.len());
                self.past.extend(seen.drain(..own).map(|(seen, _)| seen));
                self.loader = Some(loader.to_owned());
            }
            None => (self.loader, self.outcome) = (self.before.clone(), Some("same_document")),
        }
        let mut made = Heard::Nothing;
        for (loader, loaded) in seen {
            match self.heard(&loader, if loaded { "load" } else { "init" }) {
                Heard::Nothing => {}
                Heard::Lost => return Heard::Lost,
                heard => made = heard,
            }
        }
        made
    }

    /// What it answers: what became of it, and what its page answered a reading, as the page wrote it (JSON), or
    /// null.
    fn answer(&self, read: Option<&str>) -> String {
        let navigation = serde_json::json!({
            "outcome": self.outcome.unwrap_or("stopped"), "error": self.error, "began": self.began,
        });
        let read = read.filter(|read| serde_json::from_str::<serde_json::Value>(read).is_ok()).unwrap_or("null");
        format!(r#"{{"navigation":{navigation},"read":{read}}}"#)
    }

    /// Turns its page's lifecycle events off again, which it turned on: nobody reads them after it. Once; when the
    /// queue takes no more, the page is kept to be turned off at the next reading.
    fn quiet(&mut self) {
        if let (Some(page), Some(Quiet { queue, ids, loud })) = (&self.page, self.quiet.take())
            && !quieten(&queue, &ids, page)
        {
            loud.lock().unwrap().push(page.clone());
        }
    }
}

/// What turns a page's lifecycle events off: the browser's queue and ids, and where the page is kept when the queue
/// takes no more.
struct Quiet {
    queue: SyncSender<Command>,
    ids: Arc<AtomicU64>,
    loud: Arc<Mutex<Vec<String>>>,
}

/// Answers a navigation that there was no page to take anywhere (no browser yet, or none of its pages attached):
/// said, never left unanswered, so whoever asked knows nothing was navigated and may ask again.
pub fn no_page(answer: OwnedFd) {
    let said = br#"{"navigation":{"outcome":"no_page","error":null,"began":false},"read":null}"#;
    if let Err(e) = std::fs::File::from(answer).write_all(said) {
        eprintln!("browser-launch: cdp: a navigation's answer did not go: {e}");
    }
}

/// Asks the page on `session` to stop telling its lifecycle: whether the command was queued.
fn quieten(queue: &SyncSender<Command>, ids: &AtomicU64, session: &str) -> bool {
    let id = ids.fetch_add(1, Ordering::Relaxed) + 1;
    let bytes = frame(id, "Page.setLifecycleEventsEnabled", r#"{"enabled":false}"#, Some(session));
    queue.try_send(Command { bytes, opening: None, script: None }).is_ok()
}

impl Drop for Navigation {
    /// However it goes (answered, or replaced by another reading): a command to stop loading that still waits is
    /// never written, and its page's lifecycle events are turned off.
    fn drop(&mut self) {
        if let Some(stop) = &self.stop {
            give_up(stop);
        }
        self.quiet();
    }
}

impl Drop for Inspection {
    /// A script that goes (answered, given up, out of time, or replaced) is given up for its evaluation's command
    /// too, before its answer's pipe closes: a command still waiting is then never written, and one being written
    /// is waited for. Whoever sees the pipe close knows nothing of the script is written after.
    fn drop(&mut self) {
        if let Some(hold) = &self.hold {
            give_up(hold);
        }
    }
}

/// What a reading asks of each page when only the page that shows is wanted: the least `inspect.js` answers.
const SHOWN_QUERY: &str = r#"{"selector":"html","html":false,"limit":1,"text":0,"attribute":0}"#;

/// What a command of an [`Inspection`], or of a settle, answers: a page's main frame, its isolated world, or what was
/// evaluated there.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Frame,
    World,
    Read,
    /// A script's evaluation in the page that shows.
    Script,
    /// The lifecycle events of the page that shows, turned on for a navigation of it, the navigation itself, and
    /// the stop of its loading when it is given up while the browser has it.
    Lifecycle,
    Navigate,
    Stop,
}

impl Pages {
    #[cfg(test)]
    pub fn new(reply: impl Fn(u32, bool) + Send + Sync + 'static) -> Arc<Pages> {
        Pages::limited(reply, None, |_| {}, Crashes::new(|_| {}, || None))
    }

    /// Pages that keep at most `limit` tabs once Chromium has restored its own: a new one past it is closed as it opens,
    /// and `refused` hears of it; `crashes` hears of a tab whose page crashed.
    pub fn limited(
        reply: impl Fn(u32, bool) + Send + Sync + 'static,
        limit: Option<u16>,
        refused: impl Fn(u16) + Send + Sync + 'static,
        crashes: Crashes,
    ) -> Arc<Pages> {
        Arc::new(Pages {
            reply: Box::new(reply),
            limit,
            refused: Box::new(refused),
            crashes,
            debugged: std::sync::atomic::AtomicBool::new(false),
            inner: Mutex::new(Opening::default()),
        })
    }

    /// The session has developer tools: someone may debug its pages, and a debugger told to stop where a page asks
    /// for a frame would stop at the one a strict settle asks for, at an agent's key press. Strict settles then ask
    /// as the others do.
    pub fn debugged(&self) {
        self.debugged.store(true, Ordering::Relaxed);
    }

    /// A new Chromium runs: requests left for the previous one are answered as not opened, and what waited for it to
    /// settle waits no more (its keys went with it).
    fn begin(&self, generation: u64, queue: SyncSender<Command>, ids: Arc<AtomicU64>) {
        let (stale, fences): (Vec<u32>, Vec<Fence>) = {
            let mut inner = self.inner.lock().unwrap();
            let old = std::mem::replace(
                &mut *inner,
                Opening { generation, began: Some(Instant::now()), queue: Some(queue), ids, ..Opening::default() },
            );
            (old.pending.into_values().map(|(request, _)| request).collect(), old.fences)
        };
        for request in stale {
            (self.reply)(request, false);
        }
        for fence in fences {
            (fence.done)();
        }
    }

    /// Calls `done` once Chromium has handled the key events it was sent before this call, the ones a page left for
    /// the browser included (a shortcut the page did not take moves the keyboard's focus only when the page's answer
    /// comes back, and keys sent meanwhile are lost in the page). Every page and frame is asked [`MARK`], [`MARKS`]
    /// times in a row: its main thread answers the first behind the keys it was given before (the caller asks only
    /// once the browser has passed them on), having sent their answers; by the time it answers the next, the browser
    /// has had those answers and handled them, the shortcut's first, in practice: Chromium offers nothing that says so
    /// by construction without the page seeing it. A page that has just started may still be on its way to giving
    /// its main thread the keys when it is asked: `strict`, for keys the browser may act on, a page's first answer is
    /// to [`FRAME`] instead, at the cost of a frame, unless the session is [`Pages::debugged`]. A page that went away answers nothing and is
    /// waited for no more; with no Chromium, no page, or a queue that is full, there is nothing to wait for. A page
    /// that hangs, or a tab the browser froze, does not answer: the caller gives up in its own time, and a page that
    /// still owes an earlier settle its answer is not asked again until it gave it, so one such page holds up one
    /// settle, not every one after it.
    pub fn settle(&self, strict: bool, done: impl FnOnce() + Send + 'static) {
        let unasked = {
            let mut inner = self.inner.lock().unwrap();
            let waiting = inner.mark(strict && !self.debugged.load(Ordering::Relaxed));
            if waiting.is_empty() {
                Some(done)
            } else {
                inner.fences.push(Fence { waiting, more: MARKS - 1, done: Box::new(done) });
                None
            }
        };
        if let Some(done) = unasked {
            done();
        }
    }

    /// The answer `message` to command `id` of Chromium `generation`, if it was a settle's: true then.
    fn fenced(&self, generation: u64, id: u64, message: &serde_json::Value) -> bool {
        let done = {
            let mut inner = self.inner.lock().unwrap();
            if inner.generation != generation {
                return false;
            }
            let Some(at) = inner.fences.iter().position(|fence| fence.waiting.contains_key(&id)) else { return false };
            if let Some((session, step)) = inner.fences[at].waiting.remove(&id)
                && let Some((next, step)) = inner.step(&session, step, message)
            {
                inner.fences[at].waiting.insert(next, (session, step));
            }
            inner.answered(at)
        };
        if let Some(done) = done {
            done();
        }
        true
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
            let command = Command { bytes, opening: Some((id, now + PAGE_OPEN_WAIT)), script: None };
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

    /// Reads what `query` asks (JSON, see `inspect.js`) of the page shown, and writes it on `answer`, which closes with
    /// nothing on it when there is none. Every page is asked, and the one that shows answers: only a page itself knows.
    pub fn inspect(&self, query: &str, answer: OwnedFd) {
        if serde_json::from_str::<serde_json::Value>(query).is_err() {
            return;
        }
        self.read(query, None, None, answer);
    }

    /// Takes the page shown to the address `request` names (JSON: `url`, and `read`, a question as
    /// [`Pages::inspect`]'s, or null), waits for it to load (for [`NAVIGATE_LOAD`] at most), reads it when asked to,
    /// and writes on `answer` what became of the navigation and what the page answered: `{"navigation": {"outcome",
    /// "error", "began"}, "read"}`, `began` whether the browser began a new document for it, `read` the page's own
    /// answer (a JSON object), or null. The outcomes: `loaded`; `same_document` (the address is of the document
    /// shown: nothing loads); `failed` (the browser's word in `error`: the address was refused, or nothing answered
    /// there); `download` (the address is a file, which the browser downloads: the page stays as it was); `timeout`
    /// (it has not loaded yet, or the page kept going on to other documents); `no_page` (no page shows, or the
    /// browser has none yet); `ambiguous` (which page shows could not be told); `closed` (its tab closed);
    /// `crashed` (its page did); `stopped` (given up, see [`Pages::stop_navigate`]). Only a page that loaded, or
    /// stayed the document it was, is read. `answer` closes with nothing on it when there is no answer.
    ///
    /// [`NAVIGATE_LOAD`]: display_protocol::frame::NAVIGATE_LOAD
    pub fn navigate(&self, request: &str, answer: OwnedFd) {
        let Ok(request) = serde_json::from_str::<serde_json::Value>(request) else { return };
        let Some(url) = request["url"].as_str() else { return };
        let read = request.get("read").filter(|read| read.is_object()).map(serde_json::Value::to_string);
        let navigation = Navigation::to(url, read);
        // No page to ask (the browser is starting, or has none): said, so that whoever asked knows nothing was
        // navigated and may ask again.
        if let Some(answer) = self.read(SHOWN_QUERY, None, Some(navigation), answer) {
            no_page(answer);
        }
    }

    /// Gives up the navigation asked for last, if any is under way: it does not begin if it has not, and its answer
    /// says how far it got, then closes (a reading or a script is left alone). After this, nothing of it begins.
    pub fn stop_navigate(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner.inspection.as_ref().is_some_and(|inspection| inspection.navigation.is_some()) {
            inner.give_up("stopped");
        }
    }

    /// A page of Chromium `generation` said, on `session`, how far a document (`loader`) of one of its frames is.
    /// For the navigation under way of that page, what it says of its main frame's documents: the load of the one
    /// awaited takes the navigation on to its reading, or ends it; another document there (the page went on by
    /// itself, a redirect of its own) is followed, a few times, and what was read of the one before is dropped.
    /// Before the browser's answer named the document, what the page says is kept, to be known then. Its other
    /// frames only tell the time.
    fn lifecycle(&self, generation: u64, session: &str, frame: &str, loader: &str, name: &str) {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation != generation {
            return;
        }
        let inner = &mut *inner;
        let Some(navigation) = inner.inspection.as_mut().and_then(|inspection| inspection.navigation.as_mut()) else {
            return;
        };
        if navigation.page.as_deref() != Some(session) || navigation.stopping {
            return;
        }
        let heard = match navigation.frame.as_deref() == Some(frame) {
            true if navigation.replied => navigation.heard(loader, name),
            true => {
                navigation.saw(loader, name);
                Heard::Nothing
            }
            false => Heard::Nothing,
        };
        inner.heard(session, heard);
    }

    /// The page on `session` of Chromium `generation` crashed: a navigation of it ends there.
    fn page_crashed(&self, generation: u64, session: &str) {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation != generation {
            return;
        }
        let navigating = inner.inspection.as_mut().and_then(|inspection| inspection.navigation.as_mut());
        if let Some(navigation) = navigating.filter(|n| n.page.as_deref() == Some(session) && !n.stopping) {
            // Wherever it had got: a page that loaded and crashed is not read.
            if matches!(navigation.outcome, Some("loaded" | "same_document")) {
                navigation.outcome = None;
            }
            inner.give_up("crashed");
        }
    }

    /// Evaluates `script` in the page shown, in the page's own world, as a person's gesture, waiting for a promise it
    /// gives, and writes the browser's answer on `answer` (`{"result": ...}` as `Runtime.evaluate` answered, or
    /// `{"error": ...}`), which closes with nothing on it when there is none. Only in a session with developer tools
    /// ([`Pages::debugged`]): in any other, nothing an agent wrote runs on a page.
    pub fn run_script(&self, script: String, answer: OwnedFd) {
        if self.debugged.load(Ordering::Relaxed) {
            self.read(SHOWN_QUERY, Some(script), None, answer);
        }
    }

    /// Gives up the script asked for last, if any is under way: it does not start if it has not, and its answer
    /// closes with nothing more on it (a reading of the page is left alone). After this, nothing of it begins.
    pub fn stop_script(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner
            .inspection
            .as_ref()
            .is_some_and(|inspection| inspection.hold.is_some() && inspection.navigation.is_none())
        {
            inner.inspection = None;
        }
    }

    /// Begins a reading of every page: `answer` back when none could be asked (no browser yet, no page of it
    /// attached, or a queue that takes nothing), and nothing begins.
    fn read(
        &self,
        query: &str,
        script: Option<String>,
        navigation: Option<Navigation>,
        answer: OwnedFd,
    ) -> Option<OwnedFd> {
        let mut inner = self.inner.lock().unwrap();
        let Some(queue) = inner.queue.clone() else { return Some(answer) };
        // A page a navigation left telling its lifecycle is asked again to stop, while it is still there.
        let loud = std::mem::take(&mut *inner.loud.lock().unwrap());
        let still: Vec<String> = loud
            .into_iter()
            .filter(|session| inner.sessions.contains(session) && !quieten(&queue, &inner.ids, session))
            .collect();
        inner.loud.lock().unwrap().extend(still);
        let (mut waiting, mut missed) = (HashMap::new(), false);
        for session in &inner.sessions {
            let id = inner.ids.fetch_add(1, Ordering::Relaxed) + 1;
            let bytes = frame(id, "Page.getFrameTree", "{}", Some(session));
            match queue.try_send(Command { bytes, opening: None, script: None }) {
                Ok(()) => drop(waiting.insert(id, (session.clone(), Step::Frame))),
                Err(_) => missed = true,
            }
        }
        if waiting.is_empty() {
            return Some(answer);
        }
        inner.inspection = Some(Inspection {
            began: Instant::now(),
            wait: display_protocol::frame::INSPECT_WAIT,
            query: query.to_owned(),
            waiting,
            answers: Vec::new(),
            frames: HashMap::new(),
            missed,
            hold: (script.is_some() || navigation.is_some()).then(Hold::default),
            script,
            navigation,
            answer: Some(answer),
        });
        None
    }

    /// The answer to command `id` of Chromium `generation`, if it was a step of the reading under way: true then.
    fn inspecting(&self, generation: u64, id: u64, message: &serde_json::Value) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation != generation {
            return false;
        }
        let inner = &mut *inner;
        // Nobody reads its pipe past this: a page that never answered holds nothing longer.
        if inner.inspection.as_ref().is_some_and(|i| i.began.elapsed() >= i.wait) {
            inner.give_up("timeout");
        }
        let Some(inspection) = inner.inspection.as_mut() else { return false };
        let Some((session, step)) = inspection.waiting.remove(&id) else { return false };
        let result = &message["result"];
        let mut heard = Heard::Nothing;
        let next = match step {
            Step::Frame => {
                let tree = &result["frameTree"]["frame"];
                let document = tree["loaderId"].as_str().unwrap_or_default();
                if let Some(frame_id) = tree["id"].as_str() {
                    inspection.frames.insert(session.clone(), (frame_id.to_owned(), document.to_owned()));
                }
                // A navigation's page, about to be read where it went: the document there must be the one that
                // loaded, whatever the page said or did not say of another since.
                let reading = inspection.navigation.as_mut().filter(|navigation| navigation.replied);
                if let Some(navigation) = reading.filter(|_| !document.is_empty()) {
                    heard = navigation.heard(document, "init");
                }
                tree["id"].as_str().filter(|_| heard == Heard::Nothing).map(|frame_id| {
                    let params = serde_json::json!({"frameId": frame_id, "worldName": ""});
                    ("Page.createIsolatedWorld", params.to_string(), Step::World)
                })
            }
            Step::World => result["executionContextId"].as_u64().map(|context| {
                let expression = format!("({INSPECT_SCRIPT})({})", inspection.query);
                // The browser itself ends a reading that runs on (a selector over a very large page) when a
                // reading's time is over: it would keep its page from answering anyone.
                let time = display_protocol::frame::INSPECT_WAIT.as_millis() as u64;
                let params = serde_json::json!({
                    "expression": expression, "contextId": context, "returnByValue": true, "timeout": time,
                });
                ("Runtime.evaluate", params.to_string(), Step::Read)
            }),
            Step::Read => {
                if let Some(said) = result["result"]["value"].as_str() {
                    inspection.answers.push((session.clone(), said.to_owned()));
                }
                None
            }
            // The page's lifecycle events are on: the navigation is asked of it, written unless given up meanwhile,
            // and has its own time from here.
            Step::Lifecycle => inspection.navigation.as_mut().map(|navigation| {
                (inspection.began, inspection.wait) = (Instant::now(), display_protocol::frame::NAVIGATE_LOAD);
                navigation.until = inspection.began + inspection.wait;
                ("Page.navigate", serde_json::json!({"url": navigation.url}).to_string(), Step::Navigate)
            }),
            // What the browser answered the navigation: it failed, it is a download, it stays in the document
            // shown, or it began a document, whose load is waited for unless the page already said it loaded.
            Step::Navigate => {
                if let Some(navigation) = inspection.navigation.as_mut() {
                    let refused = message.get("error").and_then(|error| error["message"].as_str());
                    let failed = refused.or(result["errorText"].as_str());
                    let download = result["isDownload"] == serde_json::Value::Bool(true);
                    let (frame, loader) = (result["frameId"].as_str(), result["loaderId"].as_str());
                    if navigation.outcome.is_some() {
                        // Given up while the browser had it: this is the browser's word that it is done with it,
                        // and of whether it began a document for it meanwhile.
                        navigation.began = failed.is_none() && !download && loader.is_some();
                    } else if download {
                        navigation.outcome = Some("download");
                    } else if let Some(error) = failed {
                        (navigation.outcome, navigation.error) = (Some("failed"), Some(error.to_owned()));
                    } else {
                        heard = navigation.replied(frame, loader);
                    }
                }
                None
            }
            // The browser's answer to the stop of a navigation given up: it stops nothing after this.
            Step::Stop => None,
            // The browser's answer to the script, as it gave it: the reading's answers give way to it.
            Step::Script => {
                let said = match message.get("error") {
                    Some(error) => serde_json::json!({"error": error["message"].as_str().unwrap_or("refused")}),
                    None => serde_json::json!({"result": result}),
                };
                inspection.answers = vec![(session.clone(), said.to_string())];
                inspection.script = None;
                None
            }
        };
        let mut went_on = false;
        if let (Some((method, params, step)), Some(queue)) = (next, inner.queue.as_ref()) {
            let next_id = inner.ids.fetch_add(1, Ordering::Relaxed) + 1;
            let bytes = frame(next_id, method, &params, Some(&session));
            // A navigation is written under its hold: given up before that, it never is.
            let script = (step == Step::Navigate).then(|| inspection.hold.clone()).flatten();
            if queue.try_send(Command { bytes, opening: None, script }).is_ok() {
                inspection.waiting.insert(next_id, (session.clone(), step));
                went_on = true;
            }
        }
        // A page asked that gave no answer, or whose next step could not be asked (a full queue): what it shows is
        // not known.
        if !went_on
            && matches!(step, Step::Frame | Step::World | Step::Read)
            && !inspection.answers.iter().any(|(said, _)| *said == session)
        {
            inspection.missed = true;
        }
        inner.stopped();
        match (step, heard) {
            // It stays in the document shown, or its document had loaded already: read there. Else it loads.
            (Step::Navigate, Heard::Nothing | Heard::Loaded) => {
                inner.arrived(&session);
                inner.inspected();
            }
            (_, Heard::Nothing) => inner.inspected(),
            (_, heard) => inner.heard(&session, heard),
        }
        true
    }

    /// A page's session of Chromium `generation` began or ended.
    fn page_session(&self, generation: u64, session: &str, alive: bool) {
        let settled: Vec<Box<dyn FnOnce() + Send>> = {
            let mut inner = self.inner.lock().unwrap();
            if inner.generation != generation {
                return;
            }
            if alive {
                inner.sessions.insert(session.to_owned());
                // A page that comes while a navigation still looks for the page that shows is asked too: it may
                // be the one. If it cannot be asked, which page shows is in doubt.
                let Opening { inspection, queue, ids, .. } = &mut *inner;
                let looking = inspection.as_mut().filter(|inspection| {
                    inspection.navigation.as_ref().is_some_and(|n| n.page.is_none() && n.outcome.is_none())
                });
                if let Some(inspection) = looking {
                    let id = ids.fetch_add(1, Ordering::Relaxed) + 1;
                    let bytes = frame(id, "Page.getFrameTree", "{}", Some(session));
                    let asked = queue
                        .as_ref()
                        .is_some_and(|queue| queue.try_send(Command { bytes, opening: None, script: None }).is_ok());
                    match asked {
                        true => drop(inspection.waiting.insert(id, (session.to_owned(), Step::Frame))),
                        false => inspection.missed = true,
                    }
                }
                return;
            }
            inner.sessions.remove(session);
            // A page that went away mid-reading answers nothing: the reading goes on with the others.
            if let Some(inspection) = inner.inspection.as_mut() {
                inspection.waiting.retain(|_, (asked, _)| asked != session);
                // The page a navigation took elsewhere: the navigation ends there.
                if let Some(navigation) = inspection.navigation.as_mut()
                    && navigation.page.as_deref() == Some(session)
                    && !matches!(navigation.outcome, Some("failed" | "download"))
                {
                    (navigation.outcome, navigation.loader) = (Some("closed"), None);
                    inspection.answers.clear();
                }
                inspection.answers.retain(|(said, _)| said != session);
                inner.inspected();
            }
            // Nor any settle: one that waited for it alone goes on.
            inner.frames.remove(session);
            for fence in &mut inner.fences {
                fence.waiting.retain(|_, (asked, _)| asked != session);
            }
            let mut settled = Vec::new();
            let mut at = 0;
            while at < inner.fences.len() {
                match inner.fences[at].waiting.is_empty() {
                    // Asked again, it stays in its place; done, the next takes it.
                    true => match inner.answered(at) {
                        Some(done) => settled.push(done),
                        None if inner.fences.get(at).is_some_and(|fence| !fence.waiting.is_empty()) => at += 1,
                        None => {}
                    },
                    false => at += 1,
                }
            }
            settled
        };
        for done in settled {
            done();
        }
    }

    /// A frame's session of Chromium `generation` began: one that runs in a process of its own, with keys of its own
    /// to handle. It ends as a page's does ([`Pages::page_session`]).
    fn frame_session(&self, generation: u64, session: &str) {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation == generation {
            inner.frames.insert(session.to_owned());
        }
    }

    /// Asks the running Chromium to close the way a person quitting it does (`Browser.close`), for a saved profile's
    /// save: its windows close and its profile with them, which writes out the cookies its store still held before the
    /// browser process exits (see `profile::Closing`). Never blocks: a queue full for now is said so, to ask again.
    pub fn close_browser(&self) -> Close {
        let inner = self.inner.lock().unwrap();
        let id = inner.ids.fetch_add(1, Ordering::Relaxed) + 1;
        let bytes = frame(id, "Browser.close", "{}", None);
        match inner.queue.as_ref().map(|queue| queue.try_send(Command { bytes, opening: None, script: None })) {
            Some(Ok(())) => Close::Sent,
            Some(Err(TrySendError::Full(_))) => Close::Busy,
            Some(Err(TrySendError::Disconnected(_))) | None => Close::Gone,
        }
    }

    /// A page target of Chromium `generation` appeared or went.
    /// A new page target of Chromium `generation`, counted among its tabs unless the session already keeps all it may:
    /// then it is not, its limit is told, and the caller closes it (true). A page the display asked to open, a
    /// DevTools window, and whatever Chromium restores as it starts are never refused.
    fn refused(&self, generation: u64, target: &str, url: &str) -> bool {
        let refused = {
            let mut inner = self.inner.lock().unwrap();
            if inner.generation != generation {
                return false;
            }
            // DevTools' own window is no tab of the session's: neither counted nor refused.
            if url.starts_with("devtools://") {
                return false;
            }
            let restored = inner.began.is_none_or(|began| began.elapsed() < RESTORE_GRACE);
            let full = self.limit.filter(|&limit| inner.targets.len() >= usize::from(limit));
            let refused = full.filter(|_| !restored && inner.pending.is_empty());
            if refused.is_none() {
                inner.targets.insert(target.to_owned());
            }
            refused
        };
        if let Some(limit) = refused {
            (self.refused)(limit);
        }
        refused.is_some()
    }

    /// The process of page target `target` of Chromium `generation` ended under it with `code` (a signal's number, for
    /// one a signal ended): told for a tab the session keeps, which stays, on Chromium's error page. A frame's process,
    /// a page never counted, and an earlier Chromium's are not.
    fn crashed(&self, generation: u64, target: &str, code: Option<i64>) {
        let grew = {
            let inner = self.inner.lock().unwrap();
            if inner.generation != generation || !inner.targets.contains(target) {
                return;
            }
            // Read for every crash, so a kill counted now is never taken for a later crash's cause; and while this
            // Chromium is still the one running, so a reader of an earlier one never takes a kill of the next.
            self.crashes.grew()
        };
        (self.crashes.told)(grew && code == Some(SIGKILL));
    }

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
        let (request, opened, close) = {
            let mut inner = self.inner.lock().unwrap();
            if inner.generation != generation {
                return None;
            }
            let (request, asked) = inner.pending.remove(&id)?;
            // Too late, or a later opening was made first: nobody waits for this page any more, so it closes alone.
            let current = asked.elapsed() < PAGE_OPEN_WAIT && id > inner.shown;
            let (opened, close) = match message["result"]["targetId"].as_str() {
                Some(made) if current => {
                    inner.shown = id;
                    (true, inner.targets.iter().filter(|t| *t != made).cloned().collect())
                }
                Some(stale) => (false, vec![stale.to_owned()]),
                None => (false, Vec::new()),
            };
            (request, opened, close)
        };
        (self.reply)(request, opened);
        Some(close)
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

impl Inspection {
    /// Writes the answer of the page that shows, on a thread of its own: the pipe may wait for its reader.
    fn finish(mut self) {
        // A script that no page ran has no answer: what the pages said of themselves is not one.
        if self.script.is_some() {
            return;
        }
        let answer = match self.navigation.take() {
            // What became of the navigation, with what its page answered when it was read. Given up first, so a
            // navigation not yet written never is once its answer is on its way.
            Some(mut navigation) => {
                if let Some(hold) = &self.hold {
                    give_up(hold);
                }
                navigation.quiet();
                let read = navigation.outcome.filter(|outcome| matches!(*outcome, "loaded" | "same_document"));
                navigation.answer(read.and(self.answers.first()).map(|(_, said)| said.as_str()))
            }
            None => match shown_answer(&self.answers).cloned() {
                Some((_, answer)) => answer,
                None => return,
            },
        };
        let Some(pipe) = self.answer.take() else { return };
        let mut pipe = std::fs::File::from(pipe);
        let writing = std::thread::Builder::new().name("inspect".into()).spawn(move || {
            if let Err(e) = pipe.write_all(answer.as_bytes()) {
                eprintln!("browser-launch: cdp: a page's answer did not go: {e}");
            }
        });
        if writing.is_err() {
            eprintln!("browser-launch: cdp: no thread for a page's answer");
        }
    }
}

/// Whether a page's answer says `field` of it.
fn says(said: &(String, String), field: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(&said.1).is_ok_and(|value| value[field] == serde_json::Value::Bool(true))
}

/// The answer of a page that says it shows: each window's selected tab says it is visible, so among those, the one
/// with focus. None when no page says so.
fn visible_answer(answers: &[(String, String)]) -> Option<&(String, String)> {
    let visible = |said: &&(String, String)| says(said, "visible");
    answers.iter().filter(visible).find(|said| says(said, "focused")).or(answers.iter().find(visible))
}

/// The answer of the one page that shows, for what changes a page: the one with the keyboard; or the only one that
/// says it is visible, when every page asked answered (`missed` says one did not: it may be the one that shows).
/// None when every page answered and none says it shows; an error when which one shows cannot be told.
fn sole_visible(answers: &[(String, String)], missed: bool) -> Result<Option<&(String, String)>, ()> {
    let visible: Vec<&(String, String)> = answers.iter().filter(|said| says(said, "visible")).collect();
    let focused: Vec<&(String, String)> = visible.iter().copied().filter(|said| says(said, "focused")).collect();
    match (visible.as_slice(), focused.as_slice()) {
        (_, [one]) => Ok(Some(*one)),
        ([], _) if !missed => Ok(None),
        ([one], _) if !missed => Ok(Some(*one)),
        _ => Err(()),
    }
}

/// The answer of the page that shows, for a reading: as [`visible_answer`], or the first when no page says it shows.
fn shown_answer(answers: &[(String, String)]) -> Option<&(String, String)> {
    visible_answer(answers).or(answers.first())
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
    /// For a script's evaluation: what says whether the script was given up before this was written (see [`Hold`]).
    script: Option<Hold>,
}

/// Whether a script was given up, shared by its evaluation's command and its [`Inspection`]: the writer holds it
/// while it looks and writes, and the giving up takes it to say so. So a command is written before the script is
/// given up or not at all, and whoever is told it was given up knows nothing of it is written after.
type Hold = Arc<Mutex<Held>>;

/// Where a held command is: still queued, written to the browser, or given up before it was.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Held {
    #[default]
    Waiting,
    Written,
    GivenUp,
}

/// Gives a held command up, if it still waits: whether the browser has it instead (it was written already).
fn give_up(hold: &Hold) -> bool {
    let mut held = hold.lock().unwrap();
    if *held == Held::Waiting {
        *held = Held::GivenUp;
    }
    *held == Held::Written
}

/// The writer end of the command queue, for [`Commands`]: each command goes whole, in order.
struct Queue(SyncSender<Command>);

impl Write for Queue {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let command = Command { bytes: buf.to_vec(), opening: None, script: None };
        self.0.send(command).map_err(|_| std::io::Error::other("the writer stopped"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Attaches to Chromium's targets on `commands` and reads their download events from `events` into `sizes`, on a
/// thread of its own that keeps both ends open as long as the launcher runs; commands go through a queue a writer
/// thread empties, so whoever opens a page never blocks on the pipe (it is refused when the queue is full), and the
/// reader waits on it only once 64 commands are queued behind a stalled write.
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
        // A script given up while its evaluation waited here is never written (see [`Hold`]).
        let mut held = command.script.as_ref().map(|hold| hold.lock().unwrap());
        if held.as_ref().is_some_and(|held| **held == Held::GivenUp) {
            continue;
        }
        if pipe.write_all(&command.bytes).is_err() {
            eprintln!("browser-launch: cdp: a command did not go");
        }
        // Said while its hold is had: whoever gives it up after this knows the browser has it.
        if let Some(held) = held.as_mut() {
            **held = Held::Written;
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
                    // Past the session's tabs: closed as it opens, and never attached.
                    let url = params["targetInfo"]["url"].as_str().unwrap_or_default();
                    if let Some((pages, generation)) = &self.pages
                        && pages.refused(*generation, target, url)
                    {
                        let params = format!(r#"{{"targetId":{}}}"#, serde_json::Value::from(target));
                        self.send(None, "Target.closeTarget", &params);
                        return true;
                    }
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
                let kind = params["targetInfo"]["type"].as_str();
                let wanted = matches!(kind, Some("page" | "iframe"));
                if wanted && let Some((pages, generation)) = &self.pages {
                    match kind {
                        Some("page") => pages.page_session(*generation, session, true),
                        _ => pages.frame_session(*generation, session),
                    }
                }
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
            // A page's process ended under it; its tab stays.
            Some("Target.targetCrashed") => {
                if let (Some((pages, generation)), Some(target)) = (&self.pages, params["targetId"].as_str()) {
                    pages.crashed(*generation, target, params["errorCode"].as_i64());
                }
                true
            }
            // How far a page's document is, which a navigation of it waits for (turned on for that page then).
            Some("Page.lifecycleEvent") => {
                if let (Some((pages, generation)), Some(session), Some(frame), Some(loader), Some(name)) = (
                    &self.pages,
                    message["sessionId"].as_str(),
                    params["frameId"].as_str(),
                    params["loaderId"].as_str(),
                    params["name"].as_str(),
                ) {
                    pages.lifecycle(*generation, session, frame, loader, name);
                }
                true
            }
            // A page's process ended under it, said on the page's own session: a navigation of it ends there.
            Some("Inspector.targetCrashed") => {
                if let (Some((pages, generation)), Some(session)) = (&self.pages, message["sessionId"].as_str()) {
                    pages.page_crashed(*generation, session);
                }
                true
            }
            Some("Target.detachedFromTarget") => {
                if let Some(session) = params["sessionId"].as_str() {
                    self.sessions.remove(session);
                    if let Some((pages, generation)) = &self.pages {
                        pages.page_session(*generation, session, false);
                    }
                }
                true
            }
            Some(_) => false,
            // The answer to a page opened: its other pages close; to a step of reading a page; or to an attachment.
            None => {
                if let (Some((pages, generation)), Some(id)) = (&self.pages, message["id"].as_u64())
                    && (pages.fenced(*generation, id, message) || pages.inspecting(*generation, id, message))
                {
                    return true;
                }
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
    fn the_answer_comes_from_the_page_that_shows() {
        // Each page's answer, with the session that gave it.
        let page = |name: &str, visible: bool, focused: bool| {
            (name.to_owned(), format!(r#"{{"url":"{name}","visible":{visible},"focused":{focused}}}"#))
        };
        let (hidden, behind, front) =
            (page("hidden", false, false), page("behind", true, false), page("front", true, true));
        let answers = [hidden.clone(), behind.clone(), front.clone()];
        assert_eq!(shown_answer(&answers), Some(&front), "the window with focus, of those visible");
        assert_eq!(shown_answer(&[hidden.clone(), behind.clone()]), Some(&behind), "no focus: the visible one");
        let focused_hidden = page("focused hidden", false, true);
        assert_eq!(
            shown_answer(&[focused_hidden, behind.clone()]),
            Some(&behind),
            "a hidden page never beats a visible one"
        );
        assert_eq!(shown_answer(std::slice::from_ref(&hidden)), Some(&hidden), "none visible: what there is");
        assert_eq!(shown_answer(&[]), None);
        // A script goes only to a page that says it shows.
        assert_eq!(visible_answer(&[hidden.clone(), behind.clone(), front.clone()]), Some(&front));
        assert_eq!(visible_answer(&[hidden.clone(), behind.clone()]), Some(&behind));
        assert_eq!(visible_answer(std::slice::from_ref(&hidden)), None, "no page shows: no page runs a script");
    }

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
        hear(&sizes, 0, text);
        sizes
    }

    /// Reads `text` (messages ended by `|`) from Chromium `generation`'s pipe into `sizes`.
    fn hear(sizes: &Sizes, generation: u64, text: &str) {
        read(text.replace('|', "\0").as_bytes(), sizes, generation, &mut Commands::new(Vec::new()));
    }

    fn begin(guid: &str, name: &str) -> String {
        format!(r#"{{"method":"Page.downloadWillBegin","params":{{"guid":"{guid}","suggestedFilename":"{name}"}}}}|"#)
    }

    fn progress(guid: &str, total: u64) -> String {
        format!(
            r#"{{"method":"Page.downloadProgress","params":{{"guid":"{guid}","totalBytes":{total},"state":"inProgress"}}}}|"#
        )
    }

    fn done(guid: &str) -> String {
        format!(
            r#"{{"method":"Page.downloadProgress","params":{{"guid":"{guid}","totalBytes":5,"state":"completed"}}}}|"#
        )
    }

    /// What each crash was told as, and the kernel's count as it reads now.
    type Told = Arc<Mutex<Vec<bool>>>;
    type Count = Arc<Mutex<Option<u64>>>;

    /// Pages whose crashes go to the list returned, with the kernel's count read from the cell returned.
    fn crashing() -> (Arc<Pages>, Told, Count) {
        let (told, count) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Some(0))));
        let (heard, read) = (told.clone(), count.clone());
        let crashes = Crashes::new(move |memory| heard.lock().unwrap().push(memory), move || *read.lock().unwrap());
        let pages = Pages::limited(|_, _| {}, Some(8), |_| {}, crashes);
        let (queue, _queued) = sync_channel::<Command>(8);
        pages.begin(1, queue, Arc::new(AtomicU64::new(0)));
        assert!(!pages.refused(1, "T1", "https://one.example/"));
        (pages, told, count)
    }

    #[test]
    fn a_tabs_crash_is_told_with_memory_as_its_cause_only_when_the_kernel_killed_it() {
        let (pages, told, count) = crashing();
        // Killed, and the kernel's count grew: memory.
        *count.lock().unwrap() = Some(1);
        pages.crashed(1, "T1", Some(9));
        // Killed again with the count as it was: someone else's kill. A crash of its own: never memory.
        pages.crashed(1, "T1", Some(9));
        *count.lock().unwrap() = Some(2);
        pages.crashed(1, "T1", Some(11));
        // That growth was read with the crash before: it is not this kill's cause.
        pages.crashed(1, "T1", Some(9));
        assert_eq!(*told.lock().unwrap(), [true, false, false, false]);
        assert_eq!(pages.inner.lock().unwrap().targets.len(), 1, "the tab stays, and stays counted");
    }

    #[test]
    fn a_crash_is_told_only_for_a_tab_the_session_keeps() {
        let (pages, told, count) = crashing();
        *count.lock().unwrap() = Some(1);
        pages.crashed(1, "FRAME", Some(9));
        pages.crashed(0, "T1", Some(9));
        assert!(told.lock().unwrap().is_empty(), "a frame's process, and an earlier Chromium's page");
        // Neither read the count: the tab's own crash still finds it grown.
        pages.crashed(1, "T1", Some(9));
        assert_eq!(*told.lock().unwrap(), [true]);
        // Another Chromium runs: what its reader's predecessor still says is not told, and takes no kill of the new one.
        let (queue, _queued) = sync_channel::<Command>(8);
        pages.begin(2, queue, Arc::new(AtomicU64::new(0)));
        assert!(!pages.refused(2, "T1", "https://one.example/"));
        *count.lock().unwrap() = Some(2);
        pages.crashed(1, "T1", Some(9));
        assert_eq!(*told.lock().unwrap(), [true]);
        pages.crashed(2, "T1", Some(9));
        assert_eq!(*told.lock().unwrap(), [true, true]);
    }

    #[test]
    fn a_count_that_cannot_be_read_never_says_memory() {
        let (pages, told, count) = crashing();
        *count.lock().unwrap() = None;
        pages.crashed(1, "T1", Some(9));
        // Read again later, it is compared with what was last read, not with nothing.
        *count.lock().unwrap() = Some(3);
        pages.crashed(1, "T1", Some(9));
        assert_eq!(*told.lock().unwrap(), [false, true]);
        let unread = Crashes::new(|_| {}, || None);
        assert!(!unread.grew());
        assert_eq!(oom_kills("low 0\nhigh 0\nmax 22\noom 1\noom_kill 1\noom_group_kill 0\n"), Some(1));
        assert_eq!(oom_kills("oom 1\n"), None);
    }

    #[test]
    fn chromium_saying_a_page_crashed_reaches_its_pages() {
        let (pages, told, count) = crashing();
        *count.lock().unwrap() = Some(1);
        let mut commands = Commands::new(Vec::new());
        commands.pages = Some((pages, 1));
        let crashed = serde_json::json!({"method": "Target.targetCrashed", "params": {"targetId": "T1", "status": "killed", "errorCode": 9}});
        assert!(commands.target(&crashed));
        assert_eq!(*told.lock().unwrap(), [true]);
    }

    #[test]
    fn a_new_tab_past_the_limit_is_refused_once_chromium_restored_its_own() {
        let told = Arc::new(Mutex::new(Vec::new()));
        let heard = told.clone();
        let pages = Pages::limited(
            |_, _| {},
            Some(2),
            move |limit| heard.lock().unwrap().push(limit),
            Crashes::new(|_| {}, || None),
        );
        let (queue, _queued) = sync_channel::<Command>(8);
        pages.begin(1, queue, Arc::new(AtomicU64::new(0)));
        // While it restores, every tab it opens is its own.
        for t in ["R1", "R2", "R3"] {
            assert!(!pages.refused(1, t, "https://restored.example/"), "{t} is restored");
        }
        pages.page(1, "R3", false);
        pages.inner.lock().unwrap().began = Some(Instant::now() - RESTORE_GRACE);
        assert!(pages.refused(1, "N1", "https://new.example/"), "two kept already");
        assert!(
            !pages.refused(1, "D1", "devtools://devtools/bundled/inspector.html"),
            "DevTools is no tab, nor counted"
        );
        assert!(!pages.refused(0, "OLD", "https://new.example/"), "another Chromium's page is not counted");
        pages.page(1, "R2", false);
        assert!(!pages.refused(1, "N2", "https://new.example/"), "one closed: room for one");
        assert_eq!(*told.lock().unwrap(), vec![2], "the refusal told, with the limit");
    }

    /// The commands a settle queued: each one's id by its session, all of them of `method`.
    fn asked(queued: &Receiver<Command>, method: &str) -> HashMap<String, u64> {
        let sent = commands_in(queued);
        let session =
            |c: &str| c.split(r#""sessionId":""#).nth(1).unwrap().trim_end_matches(&['"', '}', '\0'][..]).to_owned();
        for command in &sent {
            assert!(command.contains(&format!(r#""method":"{method}""#)), "{command}");
        }
        sent.iter().map(|c| (session(c), id_of(c))).collect()
    }

    /// A page's answer for its frame: its main frame F, with `document` in it.
    fn tree(document: &str) -> serde_json::Value {
        serde_json::json!({"result": {"frameTree": {"frame": {"id": "F", "loaderId": document}}}})
    }

    /// Answers a reading's steps for every page in `sessions`: each says it shows, and has the keyboard, as `says`
    /// says of it; one it says nothing of fails its first step.
    fn found(
        pages: &Pages,
        queued: &Receiver<Command>,
        sessions: &[&str],
        says: impl Fn(&str) -> Option<(bool, bool)>,
    ) {
        let frames = asked(queued, "Page.getFrameTree");
        assert_eq!(frames.len(), sessions.len(), "{frames:?}");
        for (session, id) in &frames {
            let answer =
                says(session).map(|_| tree("OLD")).unwrap_or(serde_json::json!({"error": {"message": "gone"}}));
            assert!(pages.inspecting(1, *id, &answer));
        }
        for id in asked(queued, "Page.createIsolatedWorld").values() {
            assert!(pages.inspecting(1, *id, &serde_json::json!({"result": {"executionContextId": 7}})));
        }
        for (session, id) in asked(queued, "Runtime.evaluate") {
            let (visible, focused) = says(&session).expect("only a page that answered is asked on");
            let said = serde_json::json!({"visible": visible, "focused": focused}).to_string();
            assert!(pages.inspecting(1, id, &serde_json::json!({"result": {"result": {"value": said}}})));
        }
    }

    /// A navigation asked of pages A and B: its pages, the queue, and where its answer comes.
    fn navigating(read: Option<&str>) -> (Arc<Pages>, Receiver<Command>, std::fs::File) {
        let pages = Pages::new(|_, _| {});
        let (queue, queued) = sync_channel::<Command>(16);
        pages.begin(1, queue, Arc::new(AtomicU64::new(0)));
        pages.page_session(1, "A", true);
        pages.page_session(1, "B", true);
        let (reading, writing) = rustix::pipe::pipe().unwrap();
        let read = read.map(|selector| serde_json::json!({"selector": selector}));
        pages.navigate(&serde_json::json!({"url": "https://e.test/x", "read": read}).to_string(), writing);
        (pages, queued, std::fs::File::from(reading))
    }

    /// The one command queued, which must be `method` for page B.
    fn taken(queued: &Receiver<Command>, method: &str) -> Command {
        let command = queued.try_recv().unwrap_or_else(|_| panic!("no {method} was queued"));
        let sent = String::from_utf8(command.bytes.clone()).unwrap();
        assert!(sent.contains(&format!(r#""method":"{method}""#)) && sent.contains(r#""sessionId":"B""#), "{sent}");
        assert!(queued.try_recv().is_err(), "more than {method} was queued");
        command
    }

    fn id(command: &Command) -> u64 {
        id_of(std::str::from_utf8(&command.bytes).unwrap())
    }

    /// As [`taken`]: its id.
    fn one(queued: &Receiver<Command>, method: &str) -> u64 {
        id(&taken(queued, method))
    }

    /// The navigation's answer, once the launcher closed its pipe: nothing written reads as null. Its page's
    /// lifecycle events were turned off `quieted` times by then (once for a page that was chosen), and nothing else
    /// is asked of the browser.
    fn answered(mut pipe: std::fs::File, queued: &Receiver<Command>, quieted: usize) -> serde_json::Value {
        let mut said = String::new();
        pipe.read_to_string(&mut said).unwrap();
        let after = commands_in(queued);
        let off = |c: &&String| c.contains("setLifecycleEventsEnabled") && c.contains(r#""enabled":false"#);
        assert_eq!((after.iter().filter(off).count(), after.len()), (quieted, quieted), "{after:?}");
        serde_json::from_str(&said).unwrap_or(serde_json::Value::Null)
    }

    /// Up to the navigation's own command, queued for page B, which alone shows: the command.
    fn asked_to_navigate(pages: &Pages, queued: &Receiver<Command>) -> Command {
        found(pages, queued, &["A", "B"], |session| Some((session == "B", session == "B")));
        let lifecycle = one(queued, "Page.setLifecycleEventsEnabled");
        assert!(pages.inspecting(1, lifecycle, &serde_json::json!({"result": {}})));
        let command = taken(queued, "Page.navigate");
        let sent = String::from_utf8(command.bytes.clone()).unwrap();
        assert!(sent.contains(r#""url":"https://e.test/x""#), "{sent}");
        command
    }

    /// A command written under a hold, as the browser's writer leaves it once written: its id.
    fn written(command: &Command) -> u64 {
        *command.script.as_ref().expect("written under a hold").lock().unwrap() = Held::Written;
        id(command)
    }

    /// The browser's answer to a navigation it began a document for.
    fn began(loader: &str) -> serde_json::Value {
        serde_json::json!({"result": {"frameId": "F", "loaderId": loader}})
    }

    fn outcome(answer: &serde_json::Value) -> (&str, bool) {
        (answer["navigation"]["outcome"].as_str().unwrap_or_default(), answer["navigation"]["began"] == true)
    }

    fn under_way(pages: &Pages) -> bool {
        pages.inner.lock().unwrap().inspection.is_some()
    }

    #[test]
    fn a_navigation_takes_the_page_that_shows_and_reads_it_once_its_document_loaded() {
        let (pages, queued, pipe) = navigating(Some("a"));
        let navigation = written(&asked_to_navigate(&pages, &queued));
        // Before the browser's answer: the document before says it loaded (its events are told again as they are
        // turned on), and a frame of the page loads a document. Then the answer names the main frame's.
        pages.lifecycle(1, "B", "F", "OLD", "load");
        pages.lifecycle(1, "B", "CHILD", "L1", "load");
        assert!(pages.inspecting(1, navigation, &began("L1")));
        pages.lifecycle(1, "A", "F", "L1", "load");
        pages.lifecycle(1, "B", "CHILD", "L1", "load");
        pages.lifecycle(1, "B", "F", "OLD", "load");
        pages.lifecycle(1, "B", "F", "L1", "DOMContentLoaded");
        assert!(commands_in(&queued).is_empty(), "nothing is read before its document loaded");
        pages.lifecycle(1, "B", "F", "L1", "load");
        // Read then, on that page alone, with the question the navigation carried, within a reading's time.
        let frame = one(&queued, "Page.getFrameTree");
        assert!(pages.inspecting(1, frame, &tree("L1")));
        let world = one(&queued, "Page.createIsolatedWorld");
        assert!(pages.inspecting(1, world, &serde_json::json!({"result": {"executionContextId": 9}})));
        let read = taken(&queued, "Runtime.evaluate");
        let sent = String::from_utf8(read.bytes.clone()).unwrap();
        assert!(sent.contains(r#"{\"selector\":\"a\"}"#) && sent.contains(r#""contextId":9"#), "{sent}");
        assert!(sent.contains(r#""timeout":8000"#), "the browser ends a reading that runs on: {sent}");
        let said = serde_json::json!({"result": {"result": {"value": r#"{"url":"https://e.test/x","count":2}"#}}});
        assert!(pages.inspecting(1, id(&read), &said));
        let answer = answered(pipe, &queued, 1);
        assert_eq!(outcome(&answer), ("loaded", true));
        assert_eq!(answer["read"], serde_json::json!({"url": "https://e.test/x", "count": 2}), "as the page wrote it");

        // A reading the browser ended for its time (it answers an error, as it does a script that runs on): the
        // navigation loaded all the same, and has no reading.
        let (pages, queued, pipe) = navigating(Some("a"));
        let navigation = written(&asked_to_navigate(&pages, &queued));
        assert!(pages.inspecting(1, navigation, &began("L1")));
        pages.lifecycle(1, "B", "F", "L1", "load");
        let frame = one(&queued, "Page.getFrameTree");
        assert!(pages.inspecting(1, frame, &tree("L1")));
        let world = one(&queued, "Page.createIsolatedWorld");
        assert!(pages.inspecting(1, world, &serde_json::json!({"result": {"executionContextId": 9}})));
        let read = one(&queued, "Runtime.evaluate");
        assert!(pages.inspecting(
            1,
            read,
            &serde_json::json!({"error": {"code": -32000, "message": "Execution was terminated"}})
        ));
        let answer = answered(pipe, &queued, 1);
        assert_eq!((outcome(&answer), &answer["read"]), (("loaded", true), &serde_json::Value::Null));
    }

    #[test]
    fn a_navigation_says_what_became_of_it_and_reads_no_page_it_did_not_reach() {
        // The browser refused it, or nothing answered there: its word, and no reading.
        let (pages, queued, pipe) = navigating(Some("a"));
        let navigation = written(&asked_to_navigate(&pages, &queued));
        let refused = serde_json::json!({"result": {"frameId": "F", "loaderId": "L1", "errorText": "net::ERR_NAME_NOT_RESOLVED"}});
        assert!(pages.inspecting(1, navigation, &refused));
        let answer = answered(pipe, &queued, 1);
        assert_eq!((outcome(&answer), &answer["read"]), (("failed", false), &serde_json::Value::Null));
        assert_eq!(answer["navigation"]["error"], "net::ERR_NAME_NOT_RESOLVED");

        // A file the browser downloads: the page stays as it was, and is not read as where it went.
        let (pages, queued, pipe) = navigating(Some("a"));
        let navigation = written(&asked_to_navigate(&pages, &queued));
        let download =
            serde_json::json!({"result": {"frameId": "F", "errorText": "net::ERR_ABORTED", "isDownload": true}});
        assert!(pages.inspecting(1, navigation, &download));
        assert_eq!(outcome(&answered(pipe, &queued, 1)), ("download", false));

        // An address of the document shown (a fragment): nothing loads, no document began, and it is read at once;
        // a reading the page does not answer leaves the navigation without one.
        let (pages, queued, pipe) = navigating(Some("a"));
        let navigation = written(&asked_to_navigate(&pages, &queued));
        assert!(pages.inspecting(1, navigation, &serde_json::json!({"result": {"frameId": "F"}})));
        let frame = one(&queued, "Page.getFrameTree");
        assert!(pages.inspecting(1, frame, &serde_json::json!({"error": {"message": "gone"}})));
        let answer = answered(pipe, &queued, 1);
        assert_eq!((outcome(&answer), &answer["read"]), (("same_document", false), &serde_json::Value::Null));

        // Its document said it loaded before the browser's answer named it, and more of the page's frames loaded
        // after it than anything keeps: only the main frame's documents are kept.
        let (pages, queued, pipe) = navigating(None);
        let navigation = written(&asked_to_navigate(&pages, &queued));
        pages.lifecycle(1, "B", "F", "L7", "init");
        pages.lifecycle(1, "B", "F", "L7", "load");
        for n in 0..200 {
            pages.lifecycle(1, "B", &format!("CHILD{n}"), &format!("C{n}"), "load");
        }
        assert!(pages.inspecting(1, navigation, &began("L7")));
        assert_eq!(outcome(&answered(pipe, &queued, 1)), ("loaded", true));

        // Its tab closed while its document loaded.
        let (pages, queued, pipe) = navigating(Some("a"));
        let navigation = written(&asked_to_navigate(&pages, &queued));
        assert!(pages.inspecting(1, navigation, &began("L1")));
        pages.page_session(1, "B", false);
        assert_eq!(outcome(&answered(pipe, &queued, 1)), ("closed", true));
        // Its page crashed: while it loaded, or once it had and was being read. Another page's crash is not its own.
        for loaded in [false, true] {
            let (pages, queued, pipe) = navigating(Some("a"));
            let navigation = written(&asked_to_navigate(&pages, &queued));
            assert!(pages.inspecting(1, navigation, &began("L1")));
            if loaded {
                pages.lifecycle(1, "B", "F", "L1", "load");
                one(&queued, "Page.getFrameTree");
            }
            pages.page_crashed(1, "A");
            assert!(under_way(&pages));
            pages.page_crashed(1, "B");
            let answer = answered(pipe, &queued, 1);
            assert_eq!((outcome(&answer), &answer["read"]), (("crashed", true), &serde_json::Value::Null));
        }
    }

    #[test]
    fn a_navigation_s_answer_carries_what_its_page_read_as_it_wrote_it_and_no_larger() {
        // A reading full of quotes, near the most a reading holds: it travels as the JSON it is, never as a string
        // of it, which would nearly double it past what the display takes.
        let rows: Vec<String> = (0..2000).map(|n| format!(r#""{n}" said "{}""#, "\"".repeat(40))).collect();
        let read = serde_json::json!({"url": "https://e.test/x", "count": rows.len(), "rows": rows}).to_string();
        let mut navigation = Navigation::to("https://e.test/x", None);
        navigation.outcome = Some("loaded");
        let answer = navigation.answer(Some(&read));
        assert!(answer.len() < read.len() + 256, "{} for {}", answer.len(), read.len());
        let said: serde_json::Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(said["read"]["rows"].as_array().map(Vec::len), Some(2000));
        // What is not JSON is no reading.
        assert!(navigation.answer(Some("{not json")).ends_with(r#""read":null}"#));
    }

    #[test]
    fn a_navigation_asked_of_a_browser_with_no_page_says_so() {
        // The browser has started and none of its pages is attached yet, or it has not started: nothing is asked,
        // and the answer says no page was there, at once.
        for started in [true, false] {
            let pages = Pages::new(|_, _| {});
            let (queue, queued) = sync_channel::<Command>(16);
            if started {
                pages.begin(1, queue, Arc::new(AtomicU64::new(0)));
            }
            let (reading, writing) = rustix::pipe::pipe().unwrap();
            pages.navigate(r#"{"url":"https://e.test/x","read":null}"#, writing);
            assert_eq!(outcome(&answered(std::fs::File::from(reading), &queued, 0)), ("no_page", false));
        }
    }

    #[test]
    fn a_navigation_takes_no_page_when_which_one_shows_is_in_doubt() {
        let doubt = |says: fn(&str) -> Option<(bool, bool)>| {
            let (pages, queued, pipe) = navigating(None);
            found(&pages, &queued, &["A", "B"], says);
            match under_way(&pages) {
                true => one(&queued, "Page.setLifecycleEventsEnabled").to_string(),
                false => outcome(&answered(pipe, &queued, 0)).0.to_owned(),
            }
        };
        // No page says it shows: none is taken anywhere.
        assert_eq!(doubt(|_| Some((false, false))), "no_page");
        // Two windows show a page each and the keyboard is in neither: which one is not guessed.
        assert_eq!(doubt(|_| Some((true, false))), "ambiguous");
        // A page that did not answer may be the one that shows, or the one with the keyboard: the other is not
        // taken for it, whether it shows or not.
        assert_eq!(doubt(|session| (session == "B").then_some((true, false))), "ambiguous");
        assert_eq!(doubt(|session| (session == "B").then_some((false, false))), "ambiguous");
        // A page whose next step the queue did not take is one that did not answer.
        let (pages, queued, pipe) = navigating(None);
        let frames = asked(&queued, "Page.getFrameTree");
        let queue = pages.inner.lock().unwrap().queue.clone().unwrap();
        while queue.try_send(Command { bytes: b"{}\0".to_vec(), opening: None, script: None }).is_ok() {}
        assert!(pages.inspecting(1, frames["A"], &tree("OLD")), "its world is not asked for: the queue is full");
        while queued.try_recv().is_ok() {}
        assert!(pages.inspecting(1, frames["B"], &tree("OLD")));
        let world = one(&queued, "Page.createIsolatedWorld");
        assert!(pages.inspecting(1, world, &serde_json::json!({"result": {"executionContextId": 7}})));
        let said = serde_json::json!({"visible": true, "focused": false}).to_string();
        let read = one(&queued, "Runtime.evaluate");
        assert!(pages.inspecting(1, read, &serde_json::json!({"result": {"result": {"value": said}}})));
        assert_eq!(outcome(&answered(pipe, &queued, 0)).0, "ambiguous");
        // A page that attaches while the others are asked is asked too: it may be the one with the keyboard.
        let (pages, queued, _pipe) = navigating(None);
        let frames = asked(&queued, "Page.getFrameTree");
        pages.page_session(1, "C", true);
        let late = asked(&queued, "Page.getFrameTree");
        assert_eq!(late.keys().collect::<Vec<_>>(), ["C"], "the page that came is asked");
        let says = |session: &str| (true, session == "C");
        for (session, id) in frames.iter().chain(&late) {
            assert!(pages.inspecting(1, *id, &tree("OLD")), "{session}");
        }
        for id in asked(&queued, "Page.createIsolatedWorld").values() {
            assert!(pages.inspecting(1, *id, &serde_json::json!({"result": {"executionContextId": 7}})));
        }
        for (session, id) in asked(&queued, "Runtime.evaluate") {
            let (visible, focused) = says(&session);
            let said = serde_json::json!({"visible": visible, "focused": focused}).to_string();
            assert!(pages.inspecting(1, id, &serde_json::json!({"result": {"result": {"value": said}}})));
        }
        let sent = commands_in(&queued);
        assert!(
            sent.len() == 1 && sent[0].contains("setLifecycleEventsEnabled") && sent[0].contains(r#""sessionId":"C""#),
            "{sent:?}"
        );
        // The keyboard says which, among several that show, and whatever another did not answer.
        assert!(doubt(|session| Some((true, session == "B"))).parse::<u64>().is_ok());
        assert!(doubt(|session| (session == "B").then_some((true, true))).parse::<u64>().is_ok());
    }

    #[test]
    fn a_page_that_goes_on_to_another_document_by_itself_is_followed_there_before_it_is_read() {
        // While the first is read: what was read of it is dropped, and the next is waited for.
        let (pages, queued, pipe) = navigating(Some("a"));
        let navigation = written(&asked_to_navigate(&pages, &queued));
        assert!(pages.inspecting(1, navigation, &began("L1")));
        pages.lifecycle(1, "B", "F", "L1", "load");
        let frame = one(&queued, "Page.getFrameTree");
        pages.lifecycle(1, "B", "F", "L2", "init");
        assert!(!pages.inspecting(1, frame, &tree("L2")), "the first's reading is nobody's now");
        // What the documents before still say tells nothing of the one awaited.
        pages.lifecycle(1, "B", "F", "L1", "load");
        pages.lifecycle(1, "B", "F", "OLD", "networkIdle");
        assert!(commands_in(&queued).is_empty(), "nothing is read until the next document loaded");
        pages.lifecycle(1, "B", "F", "L2", "load");
        let frame = one(&queued, "Page.getFrameTree");
        assert!(pages.inspecting(1, frame, &serde_json::json!({"error": {"message": "gone"}})));
        assert_eq!(outcome(&answered(pipe, &queued, 1)), ("loaded", true));

        // Before the browser's answer named the first: known once it does, from what the page had said.
        let (pages, queued, pipe) = navigating(Some("a"));
        let navigation = written(&asked_to_navigate(&pages, &queued));
        for (document, name) in [("P", "load"), ("L1", "init"), ("L1", "load"), ("L2", "init")] {
            pages.lifecycle(1, "B", "F", document, name);
        }
        assert!(pages.inspecting(1, navigation, &began("L1")));
        assert!(commands_in(&queued).is_empty() && under_way(&pages), "the second is waited for");
        pages.lifecycle(1, "B", "F", "P", "load");
        pages.lifecycle(1, "B", "F", "L2", "load");
        let frame = one(&queued, "Page.getFrameTree");
        assert!(pages.inspecting(1, frame, &serde_json::json!({"error": {"message": "gone"}})));
        assert_eq!(outcome(&answered(pipe, &queued, 1)), ("loaded", true));

        // After an address of the document shown (a fragment whose page then leaves): followed all the same.
        let (pages, queued, pipe) = navigating(Some("a"));
        let navigation = written(&asked_to_navigate(&pages, &queued));
        assert!(pages.inspecting(1, navigation, &serde_json::json!({"result": {"frameId": "F"}})));
        let frame = one(&queued, "Page.getFrameTree");
        pages.lifecycle(1, "B", "F", "OLD", "networkIdle");
        pages.lifecycle(1, "B", "F", "L9", "init");
        assert!(!pages.inspecting(1, frame, &tree("L9")) && commands_in(&queued).is_empty());
        pages.lifecycle(1, "B", "F", "L9", "load");
        let frame = one(&queued, "Page.getFrameTree");
        assert!(pages.inspecting(1, frame, &tree("L9")));
        let world = one(&queued, "Page.createIsolatedWorld");
        assert!(pages.inspecting(1, world, &serde_json::json!({"error": {"message": "gone"}})));
        assert_eq!(outcome(&answered(pipe, &queued, 1)), ("loaded", true));

        // The page said nothing of the next document: the frame it is read in says which document is there, and
        // one that is not the document that loaded is waited for, never read as it.
        let (pages, queued, pipe) = navigating(Some("a"));
        let navigation = written(&asked_to_navigate(&pages, &queued));
        assert!(pages.inspecting(1, navigation, &began("L1")));
        pages.lifecycle(1, "B", "F", "L1", "load");
        let frame = one(&queued, "Page.getFrameTree");
        assert!(pages.inspecting(1, frame, &tree("L3")));
        assert!(commands_in(&queued).is_empty() && under_way(&pages), "no world is made in a document not loaded");
        pages.lifecycle(1, "B", "F", "L3", "load");
        let frame = one(&queued, "Page.getFrameTree");
        assert!(pages.inspecting(1, frame, &tree("L3")));
        one(&queued, "Page.createIsolatedWorld");
        pages.stop_navigate();
        assert_eq!(outcome(&answered(pipe, &queued, 1)), ("loaded", true));

        // One that never stops going on, each document loading before the next begins, is given up after a few:
        // not loaded, whatever loaded on the way.
        let (pages, queued, pipe) = navigating(None);
        let navigation = written(&asked_to_navigate(&pages, &queued));
        assert!(pages.inspecting(1, navigation, &began("L0")));
        for n in 1..=usize::from(MOVES) + 1 {
            assert!(under_way(&pages), "followed {n} times");
            pages.lifecycle(1, "B", "F", &format!("L{n}"), "init");
            pages.lifecycle(1, "B", "F", &format!("L{n}"), "DOMContentLoaded");
        }
        assert_eq!(outcome(&answered(pipe, &queued, 1)), ("timeout", true));
        let (pages, queued, pipe) = navigating(Some("a"));
        let navigation = written(&asked_to_navigate(&pages, &queued));
        assert!(pages.inspecting(1, navigation, &began("L0")));
        for n in 1..=usize::from(MOVES) + 1 {
            pages.lifecycle(1, "B", "F", &format!("L{}", n - 1), "load");
            one(&queued, "Page.getFrameTree");
            pages.lifecycle(1, "B", "F", &format!("L{n}"), "init");
        }
        let answer = answered(pipe, &queued, 1);
        assert_eq!((outcome(&answer), &answer["read"]), (("timeout", true), &serde_json::Value::Null));
    }

    #[test]
    fn a_navigation_given_up_or_out_of_time_says_how_far_it_got_and_begins_nothing_after() {
        // Given up before the browser was given it: the command still queued is never written.
        let (pages, queued, pipe) = navigating(Some("a"));
        let command = asked_to_navigate(&pages, &queued);
        let hold = command.script.clone().unwrap();
        assert_eq!(*hold.lock().unwrap(), Held::Waiting);
        pages.stop_navigate();
        assert_eq!(*hold.lock().unwrap(), Held::GivenUp, "the writer drops it");
        assert_eq!(outcome(&answered(pipe, &queued, 1)), ("stopped", false));
        // Its answer, late, is nobody's.
        assert!(!pages.inspecting(1, id(&command), &serde_json::json!({"result": {}})));

        // Given up while the browser has it and has not answered: the browser is told to stop loading, and nothing
        // is answered until it says what it made of the navigation. Its answer then says whether a document began.
        let aborted = serde_json::json!({"result": {"frameId": "F", "errorText": "net::ERR_ABORTED"}});
        for (reply, began_one) in [(began("L1"), true), (aborted.clone(), false)] {
            let (pages, queued, pipe) = navigating(Some("a"));
            let navigation = written(&asked_to_navigate(&pages, &queued));
            pages.stop_navigate();
            let stop = taken(&queued, "Page.stopLoading");
            assert!(under_way(&pages), "not answered before the browser is");
            // Its time running out meanwhile changes nothing of that, nor does being given up again.
            pages.inner.lock().unwrap().inspection.as_mut().unwrap().wait = Duration::ZERO;
            pages.lifecycle(1, "B", "F", "X", "networkIdle");
            pages.stop_navigate();
            assert!(under_way(&pages) && commands_in(&queued).is_empty());
            // The browser answers the navigation while the stop still waits to be written: it never is, so it
            // stops nothing a person begins once they have control.
            assert!(pages.inspecting(1, navigation, &reply));
            assert_eq!(*stop.script.as_ref().expect("under a hold of its own").lock().unwrap(), Held::GivenUp);
            assert_eq!(outcome(&answered(pipe, &queued, 1)), ("stopped", began_one));
        }
        // The browser has the stop too: its answer is waited for as well, whichever comes first.
        for stop_first in [false, true] {
            let (pages, queued, pipe) = navigating(Some("a"));
            let navigation = written(&asked_to_navigate(&pages, &queued));
            pages.stop_navigate();
            let stop = written(&taken(&queued, "Page.stopLoading"));
            let (first, second) = if stop_first { (stop, navigation) } else { (navigation, stop) };
            assert!(pages.inspecting(1, first, &aborted));
            assert!(under_way(&pages), "the browser has not answered both");
            assert!(pages.inspecting(1, second, &aborted));
            assert_eq!(outcome(&answered(pipe, &queued, 1)), ("stopped", false));
        }

        // Given up while its document loads: it began, and says so; nothing of it goes on.
        let (pages, queued, pipe) = navigating(Some("a"));
        let navigation = written(&asked_to_navigate(&pages, &queued));
        assert!(pages.inspecting(1, navigation, &began("L1")));
        pages.stop_navigate();
        assert_eq!(outcome(&answered(pipe, &queued, 1)), ("stopped", true));
        pages.lifecycle(1, "B", "F", "L1", "load");
        assert!(commands_in(&queued).is_empty());

        // Out of time while its document loads: said when the browser next says anything, of any frame.
        let (pages, queued, pipe) = navigating(Some("a"));
        let navigation = written(&asked_to_navigate(&pages, &queued));
        assert!(pages.inspecting(1, navigation, &began("L1")));
        pages.inner.lock().unwrap().inspection.as_mut().unwrap().wait = Duration::ZERO;
        pages.lifecycle(1, "B", "CHILD", "C1", "networkIdle");
        assert_eq!(outcome(&answered(pipe, &queued, 1)), ("timeout", true));

        // A load that comes late, before anything else told the time, is a load all the same.
        let (pages, queued, pipe) = navigating(None);
        let navigation = written(&asked_to_navigate(&pages, &queued));
        assert!(pages.inspecting(1, navigation, &began("L1")));
        pages.inner.lock().unwrap().inspection.as_mut().unwrap().wait = Duration::ZERO;
        pages.lifecycle(1, "B", "F", "L1", "load");
        assert_eq!(outcome(&answered(pipe, &queued, 1)), ("loaded", true));
    }

    #[test]
    fn a_navigation_that_goes_leaves_its_page_quiet_and_is_not_what_a_script_s_end_gives_up() {
        // A script given up late (its link closed after it answered) is not the navigation that came after it.
        let (pages, queued, pipe) = navigating(None);
        let navigation = written(&asked_to_navigate(&pages, &queued));
        pages.stop_script();
        assert!(pages.inspecting(1, navigation, &serde_json::json!({"result": {"frameId": "F"}})));
        assert_eq!(outcome(&answered(pipe, &queued, 1)), ("same_document", false));

        // Nor is a reading a navigation. A reading that takes a navigation's place turns its page's lifecycle
        // events off, and the navigation's command, still waiting, is never written.
        let (pages, queued, mut pipe) = navigating(None);
        let command = asked_to_navigate(&pages, &queued);
        let (_reading, writing) = rustix::pipe::pipe().unwrap();
        pages.inspect(r#"{"selector":"a"}"#, writing);
        pages.stop_navigate();
        assert!(under_way(&pages), "the reading is left alone");
        let sent = commands_in(&queued);
        let off = |c: &&String| c.contains(r#""enabled":false"#) && c.contains(r#""sessionId":"B""#);
        assert_eq!((sent.iter().filter(off).count(), sent.len()), (1, 3), "{sent:?}");
        assert_eq!(*command.script.as_ref().unwrap().lock().unwrap(), Held::GivenUp);
        let mut said = String::new();
        pipe.read_to_string(&mut said).unwrap();
        assert_eq!(said, "", "replaced, it has no answer");

        // The queue takes no more as it ends: its page is turned off at the next reading.
        let (pages, queued, mut pipe) = navigating(None);
        asked_to_navigate(&pages, &queued);
        let queue = pages.inner.lock().unwrap().queue.clone().unwrap();
        while queue.try_send(Command { bytes: b"{}\0".to_vec(), opening: None, script: None }).is_ok() {}
        pages.stop_navigate();
        pipe.read_to_string(&mut said).unwrap();
        assert!(said.contains(r#""outcome":"stopped""#), "{said}");
        assert!(commands_in(&queued).iter().all(|c| !c.contains("setLifecycleEventsEnabled")));
        let (_reading, writing) = rustix::pipe::pipe().unwrap();
        pages.inspect(r#"{"selector":"a"}"#, writing);
        let sent = commands_in(&queued);
        assert_eq!(sent.iter().filter(off).count(), 1, "{sent:?}");
        assert!(pages.inner.lock().unwrap().loud.lock().unwrap().is_empty());
    }

    #[test]
    fn a_settle_is_done_once_every_page_and_frame_answered_twice_or_went_away() {
        let settled = Arc::new(Mutex::new(Vec::new()));
        let done = |n: u32| {
            let settled = settled.clone();
            move || settled.lock().unwrap().push(n)
        };
        let ok = serde_json::json!({"result": {}});
        let pages = Pages::new(|_, _| {});
        pages.settle(false, done(0));
        assert_eq!(*settled.lock().unwrap(), [0], "no Chromium: nothing to wait for");
        let (queue, queued) = sync_channel::<Command>(8);
        let ids = Arc::new(AtomicU64::new(0));
        pages.begin(1, queue.clone(), ids.clone());
        pages.settle(false, done(1));
        assert_eq!(*settled.lock().unwrap(), [0, 1], "no page: nothing to wait for");
        pages.page_session(1, "A", true);
        pages.frame_session(1, "B");
        pages.settle(false, done(2));
        let first = asked(&queued, "Runtime.evaluate");
        assert_eq!(first.len(), 2, "the page and its frame: {first:?}");
        assert!(!pages.fenced(1, 999, &ok) && !pages.fenced(2, first["A"], &ok), "another command, another Chromium");
        assert!(pages.fenced(1, first["A"], &ok));
        assert!(commands_in(&queued).is_empty(), "the frame still to answer: nobody asked again yet");
        assert!(pages.fenced(1, first["B"], &ok));
        // Each answered once: each is asked once more.
        let second = asked(&queued, "Runtime.evaluate");
        assert_eq!(second.len(), 2, "{second:?}");
        assert_eq!(*settled.lock().unwrap(), [0, 1]);
        assert!(pages.fenced(1, second["A"], &ok));
        // The frame goes away: it answers nothing, and nothing waits for it.
        pages.page_session(1, "B", false);
        assert_eq!(*settled.lock().unwrap(), [0, 1, 2]);
        assert!(!pages.fenced(1, second["B"], &ok), "its late answer is nobody's");
        // A page that does not answer is asked no more until it does: what settles meanwhile waits for nothing.
        pages.settle(false, done(3));
        let late = asked(&queued, "Runtime.evaluate")["A"];
        pages.settle(false, done(4));
        assert!(commands_in(&queued).is_empty(), "not asked again while it owes an answer");
        assert_eq!(*settled.lock().unwrap(), [0, 1, 2, 4]);
        assert!(pages.fenced(1, late, &ok));
        let again = asked(&queued, "Runtime.evaluate")["A"];
        assert!(pages.fenced(1, again, &ok));
        assert_eq!(*settled.lock().unwrap(), [0, 1, 2, 4, 3]);
        // A page that went away between the two askings leaves nothing to ask: done.
        pages.settle(false, done(5));
        asked(&queued, "Runtime.evaluate");
        pages.page_session(1, "A", false);
        assert_eq!(*settled.lock().unwrap(), [0, 1, 2, 4, 3, 5]);
        // A new Chromium: nothing waits for the old one.
        pages.page_session(1, "C", true);
        pages.settle(false, done(6));
        asked(&queued, "Runtime.evaluate");
        pages.begin(2, queue, ids);
        assert_eq!(*settled.lock().unwrap(), [0, 1, 2, 4, 3, 5, 6]);
    }

    #[test]
    fn a_strict_settle_has_each_page_draw_a_frame_in_a_world_of_its_own_first() {
        let settled = Arc::new(Mutex::new(false));
        let pages = Pages::new(|_, _| {});
        let (queue, queued) = sync_channel::<Command>(8);
        pages.begin(1, queue, Arc::new(AtomicU64::new(0)));
        pages.page_session(1, "A", true);
        let done = settled.clone();
        pages.settle(true, move || *done.lock().unwrap() = true);
        let frame = asked(&queued, "Page.getFrameTree")["A"];
        assert!(pages.fenced(1, frame, &serde_json::json!({"result": {"frameTree": {"frame": {"id": "F"}}}})));
        let sent = commands_in(&queued);
        assert!(sent[0].contains(r#""method":"Page.createIsolatedWorld""#) && sent[0].contains(r#""frameId":"F""#));
        assert!(pages.fenced(1, id_of(&sent[0]), &serde_json::json!({"result": {"executionContextId": 7}})));
        let sent = commands_in(&queued);
        let read = &sent[0];
        assert!(read.contains(r#""method":"Runtime.evaluate""#) && read.contains(r#""contextId":7"#), "{read}");
        assert!(read.contains("requestAnimationFrame") && read.contains(r#""awaitPromise":true"#), "{read}");
        assert!(pages.fenced(1, id_of(read), &serde_json::json!({"result": {"result": {"value": 0}}})));
        // The frame drawn: then asked once more, plainly.
        assert!(!*settled.lock().unwrap());
        let plain = asked(&queued, "Runtime.evaluate")["A"];
        assert!(pages.fenced(1, plain, &serde_json::json!({"result": {}})));
        assert!(*settled.lock().unwrap());
        // A frame of the page is asked plainly even then, and a session with developer tools asks every page plainly.
        pages.frame_session(1, "B");
        pages.settle(true, || {});
        let sent = commands_in(&queued);
        let to =
            |session: &str| sent.iter().find(|c| c.contains(&format!(r#""sessionId":"{session}""#))).unwrap().clone();
        assert!(
            to("A").contains("Page.getFrameTree") && to("B").contains(r#""method":"Runtime.evaluate""#),
            "{sent:?}"
        );
        pages.page_session(1, "B", false);
        assert!(pages.fenced(1, id_of(&to("A")), &serde_json::json!({"error": {"message": "no"}})));
        let plain = asked(&queued, "Runtime.evaluate")["A"];
        assert!(pages.fenced(1, plain, &serde_json::json!({"result": {}})));
        pages.debugged();
        pages.settle(true, || {});
        let plain = asked(&queued, "Runtime.evaluate")["A"];
        assert!(pages.fenced(1, plain, &serde_json::json!({"result": {}})));
        let plain = asked(&queued, "Runtime.evaluate")["A"];
        assert!(pages.fenced(1, plain, &serde_json::json!({"result": {}})));
    }

    #[test]
    fn a_page_that_gives_no_frame_to_go_on_with_is_waited_for_no_more_at_that_step() {
        let settled = Arc::new(Mutex::new(false));
        let pages = Pages::new(|_, _| {});
        let (queue, queued) = sync_channel::<Command>(8);
        pages.begin(1, queue, Arc::new(AtomicU64::new(0)));
        pages.page_session(1, "A", true);
        let done = settled.clone();
        *settled.lock().unwrap() = false;
        pages.settle(true, move || *done.lock().unwrap() = true);
        let frame = asked(&queued, "Page.getFrameTree")["A"];
        assert!(pages.fenced(1, frame, &serde_json::json!({"error": {"message": "no"}})));
        let plain = asked(&queued, "Runtime.evaluate")["A"];
        assert!(pages.fenced(1, plain, &serde_json::json!({"result": {}})));
        assert!(*settled.lock().unwrap());
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
    fn an_older_opening_made_after_a_newer_one_closes_only_its_own_tab() {
        let answers = Arc::new(Mutex::new(Vec::new()));
        let heard = answers.clone();
        let pages = Pages::new(move |request, opened| heard.lock().unwrap().push((request, opened)));
        let (queue, queued) = sync_channel::<Command>(8);
        let ids = Arc::new(AtomicU64::new(0));
        pages.begin(1, queue.clone(), ids.clone());
        let mut commands = Commands::new(Queue(queue));
        commands.ids = ids;
        commands.pages = Some((pages.clone(), 1));
        pages.open(1, "https://sound.play/");
        pages.open(2, "https://typing.play/");
        let [older, newer] = ids_of(&queued)[..] else { panic!("two openings") };
        let made = |id: u64, target: &str| format!(r#"{{"id":{id},"result":{{"targetId":"{target}"}}}}|"#);
        let text = made(newer, "NEW") + &made(older, "OLD");
        read(text.replace('|', "\0").as_bytes(), &Sizes::default(), 1, &mut commands);
        let sent = commands_in(&queued);
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(sent[0].contains("Target.closeTarget") && sent[0].contains("OLD"), "the newer page stays");
        assert_eq!(*answers.lock().unwrap(), vec![(2, true), (1, false)]);
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
        let sizes = events(&(begin("a", "big.bin") + "{not json}|" + &"x".repeat(MAX_MESSAGE + 1) + "|"));
        assert_eq!(sizes.lookup("big.bin", false), Lookup::Wait, "no progress yet");
        let sizes = events(&(begin("a", "big.bin") + &progress("a", 4_831_838_208)));
        assert_eq!(sizes.lookup("big.bin", false), Lookup::Size(Some(4_831_838_208)));
        assert_eq!(sizes.lookup("big.bin", false), Lookup::Wait, "matched once");
        let sizes = events(&(begin("a", "big.bin") + &progress("a", 0)));
        assert_eq!(sizes.lookup("big (1).bin", false), Lookup::Size(None), "a total Chromium does not know");
        let sizes = events(&(begin("a", "big.bin") + &begin("b", "big.bin") + &progress("a", 5) + &progress("b", 6)));
        assert_eq!(sizes.lookup("big.bin", false), Lookup::Size(None), "two could be this one");
        assert_eq!(sizes.lookup("big (1).bin", false), Lookup::Size(None), "neither gives the other file its size");
        let sizes = events(&(begin("a", "big.bin") + &begin("b", "big.bin") + &progress("a", 5) + &progress("b", 6)));
        assert_eq!(sizes.lookup("big.bin", false), Lookup::Size(None));
        hear(&sizes, 0, &(begin("c", "big.bin") + &progress("c", 7)));
        assert_eq!(sizes.lookup("big (1).bin", false), Lookup::Size(None), "nor a later download's to it");
        assert_eq!(events("").lookup("big.bin", false), Lookup::Wait);
        let sizes = events(&(begin("a", "big.bin") + &progress("a", 5) + &done("a")));
        assert_eq!(
            sizes.lookup("big.bin", false),
            Lookup::Wait,
            "a finished one's file takes the size it completes with"
        );
        // Chromium finishes A and B before B's own file is announced: C's size still goes to neither file.
        let sizes = events(&(begin("a", "big.bin") + &begin("b", "big.bin") + &progress("a", 5) + &progress("b", 6)));
        assert_eq!(sizes.lookup("big.bin", false), Lookup::Size(None));
        let later = done("a") + &done("b") + &begin("c", "big.bin") + &progress("c", 7);
        hear(&sizes, 0, &later);
        assert_eq!(sizes.lookup("big (1).bin", false), Lookup::Size(None), "nor a later download's once they finished");
        assert_eq!(sizes.lookup("big (2).bin", false), Lookup::Size(None));
    }

    /// A's file is announced once Chromium finished A and began B under the same name, but before the files thread
    /// saw A's completion: it may be A's, so B's total goes to no file.
    #[test]
    fn a_download_finished_before_its_file_is_announced_keeps_a_later_one_s_size_from_it() {
        let sizes = events(
            &(begin("a", "big.bin") + &progress("a", 5) + &done("a") + &begin("b", "big.bin") + &progress("b", 6)),
        );
        assert_eq!(sizes.lookup("big.bin", false), Lookup::Size(None), "A's or B's: no size");
        assert_eq!(sizes.lookup("big (1).bin", false), Lookup::Size(None), "nor B's file A's");
    }

    /// A, not taken, is forgotten for B under the same name: A's file would match B alone, so no file is given a total
    /// until Chromium relaunches. Forgetting one taken changes nothing.
    #[test]
    fn a_download_forgotten_before_its_file_was_announced_keeps_every_size_from_the_files() {
        let others: String = (1..MAX_DOWNLOADS).map(|i| begin(&format!("o{i}"), &format!("other{i}.bin"))).collect();
        let sizes = events(&(begin("a", "report.bin") + &progress("a", 5) + &others));
        assert_eq!(sizes.lookup("report.bin", false), Lookup::Size(Some(5)), "A's file, before A is forgotten");
        hear(&sizes, 0, &(begin("b", "report.bin") + &progress("b", 6)));
        assert_eq!(sizes.lookup("report (1).bin", false), Lookup::Size(Some(6)), "A was given to its file");
        let sizes = events(&(begin("a", "report.bin") + &others + &begin("b", "report.bin") + &progress("b", 6)));
        assert_eq!(sizes.lookup("report.bin", false), Lookup::Size(None), "A's file or B's");
        assert_eq!(sizes.lookup("other1.bin", false), Lookup::Size(None), "nor any other file");
        let generation = sizes.renew();
        hear(&sizes, generation, &(begin("c", "report.bin") + &progress("c", 7)));
        assert_eq!(
            sizes.lookup("report.bin", false),
            Lookup::Size(Some(7)),
            "a relaunched Chromium's sizes are given again"
        );
        // A and B became A's file's choices: forgotten before B's file is announced, it would match C alone.
        let others: String = (2..MAX_DOWNLOADS).map(|i| begin(&format!("o{i}"), &format!("other{i}.bin"))).collect();
        let sizes = events(&(begin("a", "big.bin") + &begin("b", "big.bin") + &progress("a", 5) + &progress("b", 6)));
        hear(&sizes, 0, &others);
        assert_eq!(sizes.lookup("big.bin", false), Lookup::Size(None), "A's file, or B's");
        hear(&sizes, 0, &(begin("c", "big.bin") + &progress("c", 7) + &begin("d", "other.bin")));
        assert_eq!(sizes.lookup("big (1).bin", false), Lookup::Size(None), "B's file, or C's");
    }

    /// Makes every finished download one kept long enough, to be forgotten when next looked at.
    fn age(sizes: &Sizes) {
        for d in &mut sizes.downloads.lock().unwrap().entries {
            if d.done.is_some() {
                d.done = Instant::now().checked_sub(KEEP_DONE);
            }
        }
    }

    /// A finished download not taken is forgotten in time: its file, if still to come, would match a later one alone,
    /// so no file is given a total. Forgetting one taken changes nothing.
    #[test]
    fn a_finished_download_forgotten_before_its_file_was_announced_keeps_every_size_from_the_files() {
        let sizes = events(&(begin("a", "report.bin") + &progress("a", 5)));
        assert_eq!(sizes.lookup("report.bin", false), Lookup::Size(Some(5)));
        hear(&sizes, 0, &done("a"));
        age(&sizes);
        hear(&sizes, 0, &(begin("b", "report.bin") + &progress("b", 6)));
        assert_eq!(sizes.lookup("report (1).bin", false), Lookup::Size(Some(6)), "A was taken by its file");
        let sizes = events(&(begin("a", "report.bin") + &progress("a", 5) + &done("a")));
        age(&sizes);
        hear(&sizes, 0, &(begin("b", "report.bin") + &progress("b", 6)));
        assert_eq!(sizes.lookup("report (1).bin", false), Lookup::Size(None), "A's file, or B's");
        let sizes = events(&(begin("a", "big.bin") + &begin("b", "big.bin") + &progress("a", 5) + &progress("b", 6)));
        assert_eq!(sizes.lookup("big.bin", false), Lookup::Size(None));
        hear(&sizes, 0, &(done("a") + &done("b")));
        age(&sizes);
        hear(&sizes, 0, &(begin("c", "big.bin") + &progress("c", 7)));
        assert_eq!(sizes.lookup("big (1).bin", false), Lookup::Size(None), "B's file, or C's");
    }

    /// A file announced with no total from Chromium (complete, or after waiting) still takes the download it could be,
    /// so forgetting that one later changes nothing.
    #[test]
    fn a_file_announced_without_a_total_takes_its_download_all_the_same() {
        let sizes = events(&(begin("a", "report.bin") + &progress("a", 5) + &done("a")));
        assert_eq!(sizes.lookup("report.bin", false), Lookup::Wait, "finished: its own size is awaited");
        assert_eq!(sizes.lookup("report.bin", true), Lookup::Size(None), "complete");
        age(&sizes);
        hear(&sizes, 0, &(begin("b", "report.bin") + &progress("b", 6)));
        assert_eq!(sizes.lookup("report (1).bin", false), Lookup::Size(Some(6)));
        let sizes = events(&begin("a", "report.bin"));
        assert_eq!(sizes.lookup("report.bin", true), Lookup::Size(None), "waited enough");
        hear(&sizes, 0, &(progress("a", 5) + &done("a") + &begin("b", "report.bin") + &progress("b", 6)));
        age(&sizes);
        assert_eq!(sizes.lookup("report (1).bin", false), Lookup::Size(Some(6)));
        assert_eq!(events("").lookup("report.bin", true), Lookup::Size(None), "no download could be it");
    }

    #[test]
    fn a_relaunched_chromium_starts_with_nothing_known_and_an_old_reader_adds_nothing() {
        let text = begin("a", "big.bin") + &progress("a", 7);
        let sizes = events(&text);
        let generation = sizes.renew();
        assert_eq!(sizes.lookup("big.bin", false), Lookup::Wait, "forgotten");
        hear(&sizes, 0, &text);
        assert_eq!(sizes.lookup("big.bin", false), Lookup::Wait, "the earlier Chromium's reader is ignored");
        hear(&sizes, generation, &text);
        assert_eq!(sizes.lookup("big.bin", false), Lookup::Size(Some(7)));
    }
}
