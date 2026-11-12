//! What the agent did on screen, recorded for the administrator's replay (see `timeline` for what it did in words).
//!
//! While the agent holds a session's control, a recorder of the admin's own asks the display for a stream made to be
//! kept (`Link::Archive`, at the [`ReplayQuality`] the administrator chose: the whole screen as one AV1 video, smaller
//! and at a constant quality, a frame only when the screen changed) and keeps it as it comes: the display's own messages
//! (the configuration, video, the cursor), with the agent's pointer, each with when it came. It records nothing while a person holds control: it stops as soon as the lease
//! leaves the agent (`State::files_lease`), before the person's input can reach the page, and drops whatever comes
//! after; it starts again, from a fresh picture, once the agent holds control again.
//!
//! The stream is cut into segments of about [`SEGMENT_SPAN`]: each starts with the configuration and the cursor as they
//! stand and asks the display for a whole picture (`KeyframeRequest`), so a replay can start at any segment; a screen
//! that did not change since its last whole picture gets no other, so it costs nothing, however long it stays still. Each
//! segment is sealed (XChaCha20-Poly1305, as saved sessions are) with the session's own key into its own file under the
//! state directory's `replays/`, beside its timeline.
//!
//! They outlast the admin, behind the administrator's password. Each session's key is kept sealed for the recordings'
//! public key (`keys::seal_to`), so a session records with no one logged in; the secret half is kept wrapped by the key
//! the password derives (`keys`), so after a restart only the password opens them, as it does saved sessions.
//!
//! All of them together stay within a share of the whole disk the administrator chose ([`ReplayQuality::disk_percent`]),
//! and never leave less than [`FLOOR`] free (see [`make_room`]). Only the administrator reads them; the public demo
//! records nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use display_protocol::frame::{Archive, FrameReader, Link};
use display_protocol::{CODEC_AV1, Direction, Message};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::app::{App, Session};
use crate::keys::{DataKey, Kek, Opener, PREFIX, Sealer};

/// About how long a segment lasts: a replay starts at the beginning of one.
pub const SEGMENT_SPAN: Duration = Duration::from_secs(30);
/// How soon the first segment of a watch is sealed, so a replay of a screen that stays still has its picture.
const FIRST_SPAN: Duration = Duration::from_secs(5);
/// The most a segment holds before it is sealed anyway.
const MAX_SEGMENT: usize = 4 * 1024 * 1024;
/// How long a failed recorder waits before it watches again.
const RETRY: Duration = Duration::from_secs(2);

/// What a record of a segment is: a display message (as a viewer gets it), where the agent's pointer is (`x`, `y`,
/// big-endian `u16` each; none, both `0xffff`), or the start of a gap (control left the agent, or the recorder lost the
/// display).
pub const MESSAGE: u8 = 0;
pub const POINTER: u8 = 1;
pub const GAP: u8 = 2;

/// How sharp a replay is against what it costs on disk: the picture's size, in percent of the screen's, and its quality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayQuality {
    pub scale: u8,
    pub quality: Sharpness,
    /// The share of the whole disk replays and timelines may take, in percent.
    pub disk_percent: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sharpness {
    Low,
    Medium,
    High,
}

impl Default for ReplayQuality {
    fn default() -> ReplayQuality {
        ReplayQuality { scale: 75, quality: Sharpness::Low, disk_percent: 10 }
    }
}

impl ReplayQuality {
    /// Whether the administrator may choose it: the sizes the settings offer.
    pub fn valid(&self) -> bool {
        matches!(self.scale, 50 | 75 | 100) && (1..=90).contains(&self.disk_percent)
    }

    fn archive(self) -> Archive {
        let crf = match self.quality {
            Sharpness::Low => 55,
            Sharpness::Medium => 45,
            Sharpness::High => 35,
        };
        Archive { scale: self.scale, crf }
    }
}

/// The administrator's choice and its file, `replay-quality.json` in the state directory.
pub struct ReplayQualities {
    file: PathBuf,
    pub current: ReplayQuality,
}

impl ReplayQualities {
    /// The stored choice, the default when there is none yet.
    pub fn load(state_dir: &Path) -> Result<ReplayQualities, String> {
        let file = state_dir.join("replay-quality.json");
        let current = match crate::private::read(&file)? {
            None => ReplayQuality::default(),
            Some(bytes) => {
                let stored: ReplayQuality = crate::private::json(&file, &bytes)?;
                if !stored.valid() {
                    return Err(format!("{}: no such size", file.display()));
                }
                stored
            }
        };
        Ok(ReplayQualities { file, current })
    }

    /// The default, never written (the demo's, which records nothing).
    pub fn empty(state_dir: &Path) -> ReplayQualities {
        ReplayQualities { file: state_dir.join("replay-quality.json"), current: ReplayQuality::default() }
    }

    /// Writes `quality` and takes it, or keeps the previous one when it could not be written.
    pub fn set(&mut self, quality: ReplayQuality) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&quality).expect("a choice encodes");
        crate::private::installed(crate::private::replace(&self.file, &bytes)).map_err(String::from)?;
        self.current = quality;
        Ok(())
    }
}

/// One recording's segments: when each starts and ends (milliseconds from the recording's start) and its size sealed.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Segment {
    pub n: u32,
    pub start: u64,
    pub end: u64,
    pub size: u64,
    /// How it starts: `watch`, the first of a connection to the display (a whole picture comes first); `refresh`, a
    /// time cut, which asked the display for a whole picture as it began; `more`, a cut for size, which goes on from
    /// the segment before (see the client's `seekFrom`).
    pub starts: &'static str,
    /// It holds a whole picture: what is deleted for room ends right before one, so what is left still plays.
    pub whole: bool,
}

/// A session's archive: its recording and its timeline, sealed with its own key.
pub struct Recording {
    /// The session's id, which each file's seal is bound to.
    id: String,
    /// When it started: Unix milliseconds, as the timeline's events say when they happened.
    pub started: u64,
    /// Whether its screen is recorded: a session can say not to, and then only its timeline is kept.
    pub recorded: bool,
    dir: PathBuf,
    key: DataKey,
    inner: Mutex<Inner>,
    /// Recording now (its session runs).
    live: AtomicBool,
    /// One save of its timeline at a time; true once the last is written, as its session stopped.
    saved: Mutex<bool>,
    /// A segment was left out for lack of room: the recorder asks for a whole picture to go on from.
    left_out: AtomicBool,
    /// Bytes its index gave back when written anew, for the store's count.
    freed: AtomicU64,
}

#[derive(Default)]
struct Inner {
    /// Its segments, once read: an archive of an earlier run of the admin reads them when first asked.
    segments: Option<Vec<Segment>>,
    next: u32,
    /// Lines in its index, each sealed with its place.
    lines: u64,
    /// Why it stopped recording before its session did, if it did.
    stopped: Option<&'static str>,
    deleted: bool,
}

/// What an archive says of itself, sealed, and read as the password unlocks it.
#[derive(serde::Serialize, serde::Deserialize)]
struct Summary {
    label: String,
    started: u64,
    stopped: Option<u64>,
    recorded: bool,
}

/// A line of an archive's index, as it is kept.
#[derive(serde::Serialize, serde::Deserialize)]
struct Line {
    n: u32,
    start: u64,
    end: u64,
    size: u64,
    starts: String,
    whole: bool,
}

/// How a segment's file is called: one that holds a whole picture says so, so the oldest stretch that still plays can be
/// found and deleted while no password has opened the archive.
fn segment_file(n: u32, whole: bool) -> String {
    format!("{n:010}{}", if whole { ".whole.bin" } else { ".bin" })
}

impl Recording {
    fn new(dir: PathBuf, id: &str, recorded: bool) -> Result<Recording, String> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode_0700()
            .create(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Recording {
            id: id.to_owned(),
            started: crate::timeline::now_ms(),
            recorded,
            dir,
            key: DataKey::random(),
            inner: Mutex::new(Inner { segments: Some(Vec::new()), ..Inner::default() }),
            live: AtomicBool::new(true),
            saved: Mutex::new(false),
            left_out: AtomicBool::new(false),
            freed: AtomicU64::new(0),
        })
    }

    /// Its segments, as the API answers them: when it started, each segment, and why it stopped early, if it did.
    pub fn index(&self) -> Value {
        let mut inner = self.inner.lock().unwrap();
        let segments = self.read_segments(&mut inner);
        json!({"started": self.started, "segments": segments, "stopped": inner.stopped, "recording": self.live.load(Ordering::Relaxed)})
    }

    /// The segments of an archive kept, read from its index the first time: a line whose file was deleted for room is
    /// left out, and a line cut short by a crash ends the list.
    fn read_segments<'a>(&self, inner: &'a mut Inner) -> &'a [Segment] {
        inner.segments.get_or_insert_with(|| {
            let index = std::fs::read(self.dir.join(INDEX)).unwrap_or_default();
            let mut found: Vec<Segment> = Vec::new();
            let (mut at, mut line) = (0, 0u64);
            while let Some(length) = index.get(at..at + 4) {
                let length = u32::from_be_bytes(length.try_into().expect("four bytes")) as usize;
                let Some(sealed) = index.get(at + 4..at + 4 + length) else { break };
                let Some(opened) = open(&self.key, &format!("{}/index/{line}", self.id), sealed) else { break };
                let Ok(l) = serde_json::from_slice::<Line>(&opened) else { break };
                if self.dir.join(segment_file(l.n, l.whole)).exists() {
                    let starts = match l.starts.as_str() {
                        "watch" => "watch",
                        "refresh" => "refresh",
                        _ => "more",
                    };
                    found.push(Segment { n: l.n, start: l.start, end: l.end, size: l.size, starts, whole: l.whole });
                }
                at += 4 + length;
                line += 1;
            }
            // A crash in the middle of deleting a stretch leaves segments that cannot play without it.
            let first = found.iter().position(|s| s.whole).unwrap_or(found.len());
            found.drain(..first);
            self.compact(&found, line);
            found
        })
    }

    /// Writes the index of an archive anew when most of its lines speak of what was deleted, so it does not outgrow what
    /// is left. Only one that no longer records has its index read, and it writes no more lines.
    fn compact(&self, kept: &[Segment], lines: u64) {
        if lines < 1024 || lines < 2 * kept.len() as u64 {
            return;
        }
        let mut index = Vec::new();
        for (line, s) in kept.iter().enumerate() {
            let l =
                Line { n: s.n, start: s.start, end: s.end, size: s.size, starts: s.starts.to_owned(), whole: s.whole };
            let plain = serde_json::to_vec(&l).expect("a line encodes");
            let sealed = seal(&self.key, &format!("{}/index/{line}", self.id), &plain);
            index.extend_from_slice(&(sealed.len() as u32).to_be_bytes());
            index.extend_from_slice(&sealed);
        }
        let before = std::fs::metadata(self.dir.join(INDEX)).map_or(0, |m| m.len());
        match crate::private::replace(&self.dir.join(INDEX), &index) {
            Ok(()) => {
                self.freed.fetch_add(before.saturating_sub(index.len() as u64), Ordering::Relaxed);
            }
            Err(e) => tracing::warn!(session = %self.id, %e, "a replay's index could not be written anew"),
        }
    }

    /// Whether a segment was left out for lack of room since it was last asked.
    fn take_left_out(&self) -> bool {
        self.left_out.swap(false, Ordering::Relaxed)
    }

    /// Segment `n`'s records, opened, if it is there.
    pub fn segment(&self, n: u32) -> Option<Vec<u8>> {
        let whole = {
            let mut inner = self.inner.lock().unwrap();
            self.read_segments(&mut inner).iter().find(|s| s.n == n)?.whole
        };
        let sealed = std::fs::read(self.dir.join(segment_file(n, whole))).ok()?;
        open(&self.key, &format!("{}/{n}", self.id), &sealed)
    }

    /// Whether it was deleted, for room or by the administrator.
    pub fn deleted(&self) -> bool {
        self.inner.lock().unwrap().deleted
    }

    /// Its timeline's events changed after `after`, as it was last kept.
    pub fn timeline(&self, after: u64) -> Option<Value> {
        let sealed = std::fs::read(self.dir.join(TIMELINE)).ok()?;
        let opened = open(&self.key, &format!("{}/timeline", self.id), &sealed)?;
        let mut page: Value = serde_json::from_slice(&opened).ok()?;
        if let Some(events) = page["events"].as_array_mut() {
            events.retain(|e| e["seq"].as_u64().is_some_and(|s| s > after));
        }
        Some(page)
    }

    /// Keeps what it says of itself and its timeline (already scrubbed), so they outlast the admin.
    pub fn keep_timeline(&self, store: &Store, label: &str, stopped: Option<u64>, page: &Value) {
        let mut saved = self.saved.lock().unwrap();
        if *saved {
            return;
        }
        *saved = stopped.is_some();
        let summary = Summary { label: label.to_owned(), started: self.started, stopped, recorded: self.recorded };
        let summary = serde_json::to_vec(&summary).expect("a summary encodes");
        let page = serde_json::to_vec(page).expect("a page encodes");
        let mut grown = 0i64;
        for (name, aad, plain) in [(SUMMARY, "summary", &summary), (TIMELINE, "timeline", &page)] {
            let path = self.dir.join(name);
            let before = std::fs::metadata(&path).map_or(0, |m| m.len());
            let sealed = seal(&self.key, &format!("{}/{aad}", self.id), plain);
            match crate::private::replace(&path, &sealed) {
                Ok(()) => grown += sealed.len() as i64 - before as i64,
                Err(e) => tracing::warn!(session = %self.id, %e, "a session's timeline could not be kept"),
            }
        }
        store.charge(&self.id, grown);
    }

    /// Deletes every file for good.
    fn delete(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.deleted = true;
        inner.segments = Some(Vec::new());
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    /// Seals `plain` (segment `start`..`end`) into its file, once there is room; false once it may record no more. The
    /// room is found and the file written under the store's lock, so recordings that write at once never pass the limit
    /// together. A segment there is no room for is left out, and the recording goes on from the next whole picture.
    fn store(&self, store: &Store, start: u64, end: u64, starts: &'static str, whole: bool, plain: &[u8]) -> bool {
        let n = {
            let mut inner = self.inner.lock().unwrap();
            if inner.deleted || inner.stopped.is_some() {
                return false;
            }
            inner.next += 1;
            inner.next - 1
        };
        let sealed = seal(&self.key, &format!("{}/{n}", self.id), plain);
        let size = sealed.len() as u64;
        let line = serde_json::to_vec(&Line { n, start, end, size, starts: starts.to_owned(), whole })
            .expect("a line encodes");
        let mut all = store.holdings.lock().unwrap();
        let disk = store.disk();
        let room = make_room(&mut all, Some(&self.id), whole, size + line.len() as u64 + 128, store.percent(), disk);
        if !room {
            tracing::warn!(session = %self.id, "no room for a replay segment: it is left out");
            self.left_out.store(true, Ordering::Relaxed);
            return true;
        }
        let path = self.dir.join(segment_file(n, whole));
        let written =
            crate::private::replace(&path, &sealed).map_err(|e| e.to_string()).and_then(|()| self.append_line(&line));
        match written {
            Err(e) => {
                tracing::warn!(session = %self.id, %e, "a replay segment could not be written");
                let _ = std::fs::remove_file(&path);
                self.inner.lock().unwrap().stopped = Some("unwritten");
                return false;
            }
            Ok(added) => {
                if let Some(held) = all.get_mut(&self.id) {
                    held.bytes += size + added;
                }
            }
        }
        drop(all);
        let mut inner = self.inner.lock().unwrap();
        if inner.deleted {
            let _ = std::fs::remove_file(&path);
            return false;
        }
        if let Some(segments) = inner.segments.as_mut() {
            segments.push(Segment { n, start, end, size, starts, whole });
        }
        true
    }

    /// Appends a line to its index, each sealed alone, so a crash loses at most the last; what it added to the disk.
    fn append_line(&self, line: &[u8]) -> Result<u64, String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let lines = {
            let mut inner = self.inner.lock().unwrap();
            inner.lines += 1;
            inner.lines - 1
        };
        let sealed = seal(&self.key, &format!("{}/index/{lines}", self.id), line);
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(self.dir.join(INDEX))
            .map_err(|e| e.to_string())?;
        let mut framed = (sealed.len() as u32).to_be_bytes().to_vec();
        framed.extend_from_slice(&sealed);
        file.write_all(&framed).map_err(|e| e.to_string())?;
        Ok(framed.len() as u64)
    }

    /// Forgets the segments deleted for room, those before `n`.
    fn trimmed(&self, n: u32) {
        if let Some(segments) = self.inner.lock().unwrap().segments.as_mut() {
            segments.retain(|s| s.n >= n);
        }
    }
}

/// An archive's name for what it says of itself, its timeline, and its index; [`KEY_FILE`] holds its key.
const SUMMARY: &str = "summary";
const TIMELINE: &str = "timeline";
const INDEX: &str = "index";
const KEY_FILE: &str = "key";
/// The recordings' key pair: the public half, then the secret half wrapped by the password's key (see `keys`).
const VAULT: &str = "vault";
/// What recording never leaves free on the disk, whatever share of it the administrator chose.
const FLOOR: u64 = 1024 * 1024 * 1024;

/// The disk under the archives: its size, and what is free on it.
#[derive(Clone, Copy, Debug)]
pub struct Disk {
    pub total: u64,
    pub available: u64,
}

/// What one session's archive takes on the disk.
struct Holding {
    dir: PathBuf,
    bytes: u64,
    /// When it stopped (or last changed, for one of an earlier run of the admin); none while it runs.
    stopped: Option<u64>,
    /// Open, when this run of the admin holds its key.
    recording: Option<Arc<Recording>>,
}

/// Bytes an archive's segment files take, and the rest of its files.
fn bytes_in(dir: &Path) -> u64 {
    std::fs::read_dir(dir).map_or(0, |files| files.flatten().filter_map(|f| f.metadata().ok()).map(|m| m.len()).sum())
}

/// Its segment files, oldest first.
fn segment_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|files| {
            files.flatten().filter_map(|f| f.file_name().into_string().ok()).filter(|n| n.ends_with(".bin")).collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Deletes the oldest stretch of `held` that plays on its own, up to its next whole picture. Its last stretch goes only
/// with `last`. Whether anything went.
fn drop_oldest(id: &str, held: &mut Holding, last: bool) -> bool {
    let files = segment_files(&held.dir);
    if files.is_empty() {
        return false;
    }
    let next = files.iter().skip(1).position(|f| f.ends_with(".whole.bin")).map(|at| at + 1);
    if next.is_none() && !last {
        return false;
    }
    let mut gone = false;
    for file in &files[..next.unwrap_or(files.len())] {
        let path = held.dir.join(file);
        let size = std::fs::metadata(&path).map_or(0, |m| m.len());
        // What could not be deleted stays charged.
        match std::fs::remove_file(&path) {
            Ok(()) => {
                held.bytes = held.bytes.saturating_sub(size);
                gone = true;
            }
            Err(e) => tracing::warn!(session = %id, %e, "a replay segment could not be deleted"),
        }
    }
    if let Some(recording) = &held.recording {
        let keep = next.and_then(|at| files[at].split('.').next()?.parse().ok()).unwrap_or(u32::MAX);
        recording.trimmed(keep);
    }
    gone
}

/// Finds room for `size` more bytes of `holder`, within `percent` of the whole disk and never leaving less than [`FLOOR`]
/// free. Whichever session would hold the most, the new bytes counted, gives up its oldest stretch: one heavy session
/// eats its own past, and the small ones keep theirs. Past that, the session stopped longest ago goes whole, and a
/// running session's last stretch only goes for its own new whole picture, which needs nothing before it. Bytes that could
/// never fit cost nobody anything. Whether the bytes fit.
fn make_room(
    all: &mut BTreeMap<String, Holding>,
    holder: Option<&str>,
    whole: bool,
    size: u64,
    percent: u8,
    disk: Disk,
) -> bool {
    for held in all.values_mut() {
        if let Some(recording) = &held.recording {
            held.bytes = held.bytes.saturating_sub(recording.freed.swap(0, Ordering::Relaxed));
        }
    }
    let used: u64 = all.values().map(|h| h.bytes).sum();
    let share = disk.total / 100 * u64::from(percent);
    let limit = share.min((used + disk.available).saturating_sub(FLOOR));
    if size > limit {
        return false;
    }
    loop {
        let used: u64 = all.values().map(|h| h.bytes).sum();
        if used + size <= limit {
            return true;
        }
        let largest = all
            .iter()
            .max_by_key(|(id, h)| h.bytes + if Some(id.as_str()) == holder { size } else { 0 })
            .map(|(id, _)| id.clone());
        if let Some(id) = largest {
            let own = Some(id.as_str()) == holder;
            let held = all.get_mut(&id).expect("just found");
            if drop_oldest(&id, held, own && whole) {
                continue;
            }
        }
        let oldest = all
            .iter()
            .filter(|(id, h)| Some(id.as_str()) != holder && h.stopped.is_some())
            .min_by_key(|(_, h)| h.stopped)
            .map(|(id, _)| id.clone());
        let Some(id) = oldest else { return false };
        let held = all.remove(&id).expect("just found");
        match &held.recording {
            Some(recording) => recording.delete(),
            None => {
                let _ = std::fs::remove_dir_all(&held.dir);
            }
        }
        crate::app::list_changed();
    }
}

/// Every session's archive, and the key pair that seals them.
pub struct Store {
    dir: PathBuf,
    holdings: Mutex<BTreeMap<String, Holding>>,
    /// The share of the disk they may take, in percent.
    percent: AtomicU8,
    /// The recordings' public key, and the secret one once the password opened it.
    public: Mutex<Option<[u8; 32]>>,
    secret: Mutex<Option<DataKey>>,
}

impl Store {
    /// The archives under `state_dir`'s `replays/`. One with no key file can never open again (the admin stopped before
    /// any password made the key pair) and goes now; the others wait for the password.
    pub fn open(state_dir: &Path) -> Store {
        let dir = state_dir.join("replays");
        let mut holdings = BTreeMap::new();
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let (Some(id), true) = (entry.file_name().to_str().map(str::to_owned), path.is_dir()) else { continue };
            if !path.join(KEY_FILE).exists() {
                let _ = std::fs::remove_dir_all(&path);
                continue;
            }
            let changed = entry.metadata().ok().and_then(|m| m.modified().ok());
            let changed =
                changed.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64);
            holdings.insert(id, Holding { bytes: bytes_in(&path), dir: path, stopped: Some(changed), recording: None });
        }
        let public = std::fs::read(dir.join(VAULT)).ok().and_then(|v| v.get(..32)?.try_into().ok());
        Store {
            dir,
            holdings: Mutex::new(holdings),
            percent: AtomicU8::new(ReplayQuality::default().disk_percent),
            public: Mutex::new(public),
            secret: Mutex::new(None),
        }
    }

    /// A new session's archive; its key is sealed for the password as it starts, if a password made the key pair yet.
    pub fn create(&self, id: &str, recorded: bool) -> Result<Arc<Recording>, String> {
        let recording = Arc::new(Recording::new(self.dir.join(id), id, recorded)?);
        // Listed before the public key is read: an unlock either finds it here to seal, or made the key this reads.
        let mut all = self.holdings.lock().unwrap();
        let public = *self.public.lock().unwrap();
        if let Some(public) = public {
            write_key(&recording, &public)?;
        }
        all.insert(
            id.to_owned(),
            Holding {
                bytes: bytes_in(&recording.dir),
                dir: recording.dir.clone(),
                stopped: None,
                recording: Some(recording.clone()),
            },
        );
        Ok(recording)
    }

    /// Its session stopped.
    pub fn stopped(&self, id: &str) {
        if let Some(held) = self.holdings.lock().unwrap().get_mut(id) {
            held.stopped = Some(crate::timeline::now_ms());
        }
    }

    /// The administrator deleted it.
    pub fn forget(&self, id: &str) {
        let Some(held) = self.holdings.lock().unwrap().remove(id) else { return };
        match &held.recording {
            Some(recording) => recording.delete(),
            None => {
                let _ = std::fs::remove_dir_all(&held.dir);
            }
        }
    }

    /// Takes the password's key: makes the key pair the first time, seals the keys of what runs with it, and opens every
    /// archive an earlier run of the admin kept, each returned to be listed. A password set anew, after the old one was
    /// lost, opens none of them, which then can never open again and go.
    pub fn unlock(&self, kek: &Kek) -> Vec<(String, Arc<Recording>, Option<u64>, String)> {
        let mut secret_held = self.secret.lock().unwrap();
        if secret_held.is_some() {
            return Vec::new();
        }
        let path = self.dir.join(VAULT);
        let stored = std::fs::read(&path).ok();
        let secret = stored.as_ref().and_then(|v| crate::keys::unwrap(kek, VAULT, v.get(32..)?));
        let secret = match secret {
            Some(secret) => secret,
            None => {
                if stored.is_some() {
                    tracing::warn!("the password set anew opens no replay: they go");
                    let mut all = self.holdings.lock().unwrap();
                    let gone: Vec<String> =
                        all.iter().filter(|(_, h)| h.recording.is_none()).map(|(id, _)| id.clone()).collect();
                    for id in gone {
                        if let Some(held) = all.remove(&id) {
                            let _ = std::fs::remove_dir_all(&held.dir);
                        }
                    }
                }
                let secret = DataKey::random();
                let public = crate::keys::public_of(&secret);
                let mut vault = public.to_vec();
                vault.extend_from_slice(&crate::keys::wrap(kek, VAULT, &secret));
                let _ = std::fs::DirBuilder::new().recursive(true).mode_0700().create(&self.dir);
                if let Err(e) = crate::private::replace(&path, &vault) {
                    tracing::warn!(%e, "the replays' key pair could not be written");
                    return Vec::new();
                }
                *self.public.lock().unwrap() = Some(public);
                secret
            }
        };
        // The key the password opened decides, whatever public key the file stood beside: what runs is sealed for it again.
        let public = crate::keys::public_of(&secret);
        let tampered = stored.as_ref().is_some_and(|v| v.get(..32) != Some(&public[..]));
        if tampered {
            tracing::warn!("the replays' public key was not the password's: it is written anew");
            let mut vault = public.to_vec();
            vault.extend_from_slice(&crate::keys::wrap(kek, VAULT, &secret));
            if let Err(e) = crate::private::replace(&path, &vault) {
                tracing::warn!(%e, "the replays' key pair could not be written");
                return Vec::new();
            }
        }
        *self.public.lock().unwrap() = Some(public);
        let (mut opened, mut unreadable) = (Vec::new(), Vec::new());
        let mut all = self.holdings.lock().unwrap();
        for (id, held) in all.iter_mut() {
            // What already runs: it may have started before any key pair, or sealed for one that no password opens now.
            if let Some(recording) = &held.recording {
                let _ = write_key(recording, &public);
                continue;
            }
            // Nothing in it opens, the password being right: nothing ever will.
            let Some((recording, summary)) = reopen(&held.dir, id, &secret) else {
                unreadable.push(id.clone());
                continue;
            };
            let recording = Arc::new(recording);
            held.recording = Some(recording.clone());
            let stopped = summary.stopped.or(held.stopped);
            held.stopped = stopped;
            opened.push((id.clone(), recording, stopped, summary.label));
        }
        for id in unreadable {
            if let Some(held) = all.remove(&id) {
                tracing::warn!(session = %id, "a replay that does not open goes");
                let _ = std::fs::remove_dir_all(&held.dir);
            }
        }
        drop(all);
        *secret_held = Some(secret);
        opened
    }

    /// The recordings' public key, once a password made the pair: what is kept for the password alone is sealed for it
    /// (see `resume`).
    pub fn public(&self) -> Option<[u8; 32]> {
        *self.public.lock().unwrap()
    }

    /// A key sealed for the public key (`keys::seal_to`), once the password opened the secret one.
    pub fn open_key(&self, sealed: &[u8]) -> Option<DataKey> {
        self.secret.lock().unwrap().as_ref().and_then(|secret| crate::keys::open_from(secret, sealed))
    }

    /// Whether the password opened them.
    pub fn unlocked(&self) -> bool {
        self.secret.lock().unwrap().is_some()
    }

    /// Archives kept that only the password opens.
    pub fn locked(&self) -> usize {
        self.holdings.lock().unwrap().values().filter(|h| h.recording.is_none()).count()
    }

    /// What they take on the disk, and the disk.
    pub fn usage(&self) -> (u64, Disk) {
        (self.holdings.lock().unwrap().values().map(|h| h.bytes).sum(), self.disk())
    }

    pub fn set_percent(&self, percent: u8) {
        self.percent.store(percent, Ordering::Relaxed);
    }

    fn percent(&self) -> u8 {
        self.percent.load(Ordering::Relaxed)
    }

    fn disk(&self) -> Disk {
        let stat =
            rustix::fs::statvfs(&self.dir).or_else(|_| rustix::fs::statvfs(self.dir.parent().unwrap_or(&self.dir)));
        stat.map_or(Disk { total: 0, available: 0 }, |s| Disk {
            total: s.f_blocks.saturating_mul(s.f_frsize),
            available: s.f_bavail.saturating_mul(s.f_frsize),
        })
    }

    /// Holds the limit with no write to do it: the disk filled from elsewhere, or the administrator lowered it.
    pub fn sweep(&self) {
        let disk = self.disk();
        make_room(&mut self.holdings.lock().unwrap(), None, false, 0, self.percent(), disk);
    }

    fn charge(&self, id: &str, grown: i64) {
        if let Some(held) = self.holdings.lock().unwrap().get_mut(id) {
            held.bytes = held.bytes.saturating_add_signed(grown);
        }
    }
}

/// Seals `recording`'s key for the password.
fn write_key(recording: &Recording, public: &[u8; 32]) -> Result<(), String> {
    crate::private::replace(&recording.dir.join(KEY_FILE), &crate::keys::seal_to(public, &recording.key))
        .map_err(|e| e.to_string())
}

/// An archive an earlier run of the admin kept, opened with the recordings' secret key.
fn reopen(dir: &Path, id: &str, secret: &DataKey) -> Option<(Recording, Summary)> {
    let key = crate::keys::open_from(secret, &std::fs::read(dir.join(KEY_FILE)).ok()?)?;
    let sealed = std::fs::read(dir.join(SUMMARY)).ok()?;
    let summary: Summary = serde_json::from_slice(&open(&key, &format!("{id}/summary"), &sealed)?).ok()?;
    let recording = Recording {
        id: id.to_owned(),
        started: summary.started,
        recorded: summary.recorded,
        dir: dir.to_path_buf(),
        key,
        inner: Mutex::default(),
        live: AtomicBool::new(false),
        saved: Mutex::new(true),
        left_out: AtomicBool::new(false),
        freed: AtomicU64::new(0),
    };
    Some((recording, summary))
}

/// Seals `plain` for `id`: the prefix, then its chunks.
fn seal(key: &DataKey, id: &str, plain: &[u8]) -> Vec<u8> {
    let (mut sealer, prefix) = Sealer::new(key, id);
    let mut sealed = prefix.to_vec();
    sealer.push(plain, &mut sealed);
    sealer.finish(&mut sealed);
    sealed
}

/// Opens what [`seal`] sealed for `id`, if it opens whole.
fn open(key: &DataKey, id: &str, sealed: &[u8]) -> Option<Vec<u8>> {
    let units = crate::keys::units(sealed.len() as u64)?;
    let prefix: [u8; PREFIX] = sealed.get(..PREFIX)?.try_into().ok()?;
    let mut opener = Some(Opener::new(key, id, &prefix));
    let mut plain = Vec::with_capacity(sealed.len());
    for (i, (offset, length)) in units.iter().enumerate() {
        let chunk = &sealed[*offset as usize..*offset as usize + length];
        let opened =
            if i + 1 == units.len() { opener.take()?.open_last(chunk)? } else { opener.as_mut()?.open(chunk)? };
        plain.extend_from_slice(&opened);
    }
    Some(plain)
}

/// One segment being recorded: its records, and what it starts with.
struct Builder {
    start: u64,
    began: Instant,
    records: Vec<u8>,
    media: bool,
    /// The screen changed since this segment's whole picture.
    changed: bool,
    /// It holds a whole picture.
    whole: bool,
    /// How it starts (see [`Segment::starts`]).
    starts: &'static str,
}

impl Builder {
    fn new(start: u64, starts: &'static str) -> Builder {
        Builder {
            start,
            began: Instant::now(),
            records: Vec::new(),
            media: false,
            changed: false,
            whole: false,
            starts,
        }
    }

    /// Appends a record: when (milliseconds from the recording's start), its kind, its length and its bytes.
    fn push(&mut self, at: u64, kind: u8, bytes: &[u8]) {
        self.records.extend_from_slice(&(at.min(u64::from(u32::MAX)) as u32).to_be_bytes());
        self.records.push(kind);
        self.records.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        self.records.extend_from_slice(bytes);
        self.media |= kind == MESSAGE;
    }
}

/// Records `session` while its agent holds control, until it stops (see the module's docs).
pub fn spawn(app: Arc<App>, session: Arc<Session>, recording: Arc<Recording>) {
    tokio::spawn(async move {
        let mut lease = session.state.lock().unwrap().files_lease.subscribe();
        let mut ended = session.ended.subscribe();
        loop {
            if *ended.borrow() {
                break;
            }
            if lease.borrow().is_none() {
                tokio::select! {
                    _ = lease.changed() => continue,
                    _ = ended.changed() => continue,
                }
            }
            if let Err(e) = capture(&app, &session, &recording, &mut lease, &mut ended).await {
                tracing::warn!(session = %session.id, %e, "the replay recorder lost the display");
                tokio::time::sleep(RETRY).await;
            }
            if recording.inner.lock().unwrap().stopped.is_some() {
                break;
            }
        }
        recording.live.store(false, Ordering::Relaxed);
    });
}

/// The configuration and the cursor as they stand: what every later segment of a connection starts with.
#[derive(Default)]
struct Standing {
    epoch: Option<u32>,
    config: Option<Vec<u8>>,
    cursor: Option<Vec<u8>>,
}

/// Watches the display while the agent holds control at the lease epoch it held when the watch began: returns once
/// that lease changes in any way (a person took control, even if it came back since), the session ends, or recording
/// may not go on. Nothing received after the change is kept.
async fn capture(
    app: &App,
    session: &Session,
    recording: &Arc<Recording>,
    lease: &mut tokio::sync::watch::Receiver<Option<u32>>,
    ended: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    let pinned = *lease.borrow_and_update();
    if pinned.is_none() {
        return Ok(());
    }
    // Whether the agent still holds the lease it held as this watch began.
    let held = |lease: &tokio::sync::watch::Receiver<Option<u32>>| *lease.borrow() == pinned;
    let path = app.config.streams_dir.join(&session.id).join("display.sock");
    let display = UnixStream::connect(&path).await.map_err(|e| e.to_string())?;
    let (mut rx, mut tx) = display.into_split();
    let viewer = app.next_viewer.fetch_add(1, Ordering::Relaxed) + 1;
    let archive = app.replay_quality.lock().unwrap().current.archive();
    let hello = Message::Hello { version: display_protocol::VERSION, codecs: CODEC_AV1 };
    for link in [Link::Attach { viewer }, Link::Archive(archive), Link::Relay(hello)] {
        tx.write_all(&link.encode()).await.map_err(|e| e.to_string())?;
    }
    let at = || crate::timeline::now_ms().saturating_sub(recording.started);
    let mut segment = Builder::new(at(), "watch");
    let mut standing = Standing::default();
    let mut places = session.agent.places();
    let mut frames = FrameReader::new(Direction::ServerToClient);
    let mut chunk = vec![0u8; 64 * 1024];
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let outcome: Result<(), String> = loop {
        // A cut, and how the next segment starts: a time cut asks for a whole picture; a cut for size does not (a
        // picture larger than a segment would otherwise start over before it is whole).
        let mut cut: Option<&'static str> = None;
        tokio::select! {
            _ = lease.changed() => {
                if !held(lease) {
                    break Ok(());
                }
            }
            _ = ended.changed() => break Ok(()),
            read = rx.read(&mut chunk) => {
                let n = match read {
                    Ok(0) => break Err("the display closed the stream".to_owned()),
                    Ok(n) => n,
                    Err(e) => break Err(e.to_string()),
                };
                if let Err(e) = frames.push(&chunk[..n]) {
                    break Err(format!("{e:?}"));
                }
                let mut changed = false;
                while let Ok(Some(body)) = frames.next_body() {
                    // Nothing once the lease moved, even if it came back: what follows may show a person's input.
                    if !held(lease) {
                        changed = true;
                        break;
                    }
                    let Ok(Link::Relay(message)) = Link::decode(&body, Direction::ServerToClient) else { continue };
                    if let Some(revision) = message.media_revision() {
                        let ack = Link::Relay(Message::MediaAck { revision }).encode();
                        if let Err(e) = tx.write_all(&ack).await {
                            return finish(app, recording, segment, at()).await.and(Err(e.to_string()));
                        }
                    }
                    let bytes = &body[1..];
                    match &message {
                        Message::Config { epoch, .. } => {
                            standing.epoch = Some(*epoch);
                            standing.config = Some(bytes.to_vec());
                        }
                        Message::Cursor { .. } => standing.cursor = Some(bytes.to_vec()),
                        Message::Video { keyframe, .. } => {
                            segment.changed |= !keyframe;
                            segment.whole |= keyframe;
                        }
                        _ => continue,
                    }
                    segment.push(at(), MESSAGE, bytes);
                }
                if changed {
                    break Ok(());
                }
                if segment.records.len() >= MAX_SEGMENT {
                    cut = Some("more");
                }
            }
            _ = places.changed() => {
                let place = *places.borrow_and_update();
                let (x, y) = place.unwrap_or((u16::MAX, u16::MAX));
                let mut bytes = [0u8; 4];
                bytes[..2].copy_from_slice(&x.to_be_bytes());
                bytes[2..].copy_from_slice(&y.to_be_bytes());
                segment.push(at(), POINTER, &bytes);
            }
            _ = tick.tick() => {
                // A new choice applies at once: the next watch asks for it.
                if app.replay_quality.lock().unwrap().current.archive() != archive {
                    break Ok(());
                }
                let first = segment.starts == "watch";
                if segment.began.elapsed() >= if first { FIRST_SPAN } else { SEGMENT_SPAN } {
                    // A whole picture only for a screen that changed since the last one: what follows a picture left
                    // as it was goes on from it.
                    cut = Some(if segment.changed && !first { "refresh" } else { "more" });
                }
            }
        }
        if let Some(starts) = cut
            && segment.media
            && let (Some(epoch), Some(config)) = (standing.epoch, standing.config.as_ref())
        {
            // The next segment starts with the configuration and the cursor as they stand, then (a time cut) a whole
            // picture.
            let mut next = Builder::new(at(), starts);
            next.push(at(), MESSAGE, config);
            if let Some(cursor) = &standing.cursor {
                next.push(at(), MESSAGE, cursor);
            }
            next.media = false;
            let done = std::mem::replace(&mut segment, next);
            if starts == "refresh" {
                let request = Link::Relay(Message::KeyframeRequest { epoch }).encode();
                if let Err(e) = tx.write_all(&request).await {
                    break Err(e.to_string());
                }
            }
            let end = at();
            let kept = keep(app, recording, &done, end).await;
            session.agent.timeline.touch();
            if !kept {
                return Ok(());
            }
            // What follows a segment left out has nothing to go on from: a whole picture now, not at the next cut.
            if recording.take_left_out() {
                let request = Link::Relay(Message::KeyframeRequest { epoch }).encode();
                if let Err(e) = tx.write_all(&request).await {
                    break Err(e.to_string());
                }
            }
        }
    };
    finish(app, recording, segment, at()).await?;
    session.agent.timeline.touch();
    outcome
}

/// Keeps the last segment of a watch, ended by a gap, when it holds anything of the screen.
async fn finish(app: &App, recording: &Arc<Recording>, mut segment: Builder, end: u64) -> Result<(), String> {
    segment.push(end, GAP, &[]);
    if segment.media {
        keep(app, recording, &segment, end).await;
    }
    Ok(())
}

/// Seals and stores a segment off the async workers; false once the recording may not go on.
async fn keep(app: &App, recording: &Arc<Recording>, segment: &Builder, end: u64) -> bool {
    let (store, recording) = (app.replays.clone(), recording.clone());
    let (start, starts, whole, records) = (segment.start, segment.starts, segment.whole, segment.records.clone());
    tokio::task::spawn_blocking(move || recording.store(&store, start, end, starts, whole, &records))
        .await
        .unwrap_or(false)
}

/// `DirBuilder::mode(0o700)` as a builder step.
trait Private {
    fn mode_0700(&mut self) -> &mut Self;
}

impl Private for std::fs::DirBuilder {
    fn mode_0700(&mut self) -> &mut Self {
        use std::os::unix::fs::DirBuilderExt;
        self.mode(0o700)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quality_chosen_is_kept_and_only_a_size_the_settings_offer_is_taken() {
        let dir = crate::private::test_dir("replay-quality");
        assert_eq!(ReplayQualities::load(&dir).unwrap().current, ReplayQuality::default());
        let sharp = ReplayQuality { scale: 100, quality: Sharpness::High, disk_percent: 20 };
        ReplayQualities::load(&dir).unwrap().set(sharp).unwrap();
        assert_eq!(ReplayQualities::load(&dir).unwrap().current, sharp);
        assert_eq!(sharp.archive(), Archive { scale: 100, crf: 35 });
        assert!(!ReplayQuality { scale: 60, ..ReplayQuality::default() }.valid());
        assert!(!ReplayQuality { disk_percent: 0, ..ReplayQuality::default() }.valid());
        assert!(!ReplayQuality { disk_percent: 91, ..ReplayQuality::default() }.valid());
        for quality in
            [ReplayQuality::default(), sharp, ReplayQuality { scale: 50, quality: Sharpness::Medium, disk_percent: 1 }]
        {
            assert!(quality.archive().valid(), "the display takes {quality:?}");
        }
    }

    #[test]
    fn a_segment_opens_only_whole_and_only_as_its_own() {
        let key = DataKey::random();
        let plain: Vec<u8> = (0..200_000u32).map(|i| (i * 7) as u8).collect();
        let sealed = seal(&key, "abc/0", &plain);
        assert_eq!(open(&key, "abc/0", &sealed).as_deref(), Some(&plain[..]));
        assert_eq!(open(&key, "abc/1", &sealed), None, "bound to its session and number");
        assert_eq!(open(&DataKey::random(), "abc/0", &sealed), None);
        assert_eq!(open(&key, "abc/0", &sealed[..sealed.len() - 1]), None, "cut short");
    }

    #[test]
    fn records_are_time_kind_length_and_bytes() {
        let mut segment = Builder::new(5, "watch");
        segment.push(7, POINTER, &[0, 1, 0, 2]);
        assert!(!segment.media);
        segment.push(9, MESSAGE, b"m");
        assert!(segment.media);
        assert_eq!(
            segment.records,
            [0, 0, 0, 7, POINTER, 0, 0, 0, 4, 0, 1, 0, 2, 0, 0, 0, 9, MESSAGE, 0, 0, 0, 1, b'm']
        );
    }

    const MIB: u64 = 1024 * 1024;

    /// A plenty of free disk, of `total` bytes.
    fn disk(total: u64) -> Disk {
        Disk { total, available: total }
    }

    /// `n` sealed stretches of `size` bytes each for `id`, each from a whole picture.
    fn fill(store: &Store, id: &str, stretches: u32, size: usize) -> Arc<Recording> {
        let recording = store.create(id, true).unwrap();
        recording.live.store(false, Ordering::Relaxed);
        store.stopped(id);
        for i in 0..stretches {
            let (start, end) = (u64::from(i) * 10, u64::from(i) * 10 + 10);
            assert!(recording.store(store, start, end, "refresh", true, &vec![7; size]));
        }
        recording
    }

    #[test]
    fn a_new_heavy_session_eats_its_own_past_and_the_old_ones_keep_theirs() {
        let dir = crate::private::test_dir("replays-fair");
        let store = Store::open(&dir);
        // 1% of it is 100 MiB, with the floor far off.
        let total = 10_000 * MIB;
        let old = fill(&store, "aaaaaaaaaaaa", 4, MIB as usize);
        let heavy = store.create("bbbbbbbbbbbb", true).unwrap();
        let mut all = store.holdings.lock().unwrap();
        // Up to the limit of 100 MiB, nobody gives up anything.
        assert!(make_room(&mut all, Some("bbbbbbbbbbbb"), true, 90 * MIB, 1, disk(total)));
        drop(all);
        assert_eq!(old.index()["segments"].as_array().unwrap().len(), 4);
        for i in 0..4 {
            let (start, end) = (i * 10, i * 10 + 10);
            assert!(heavy.store(&store, start, end, "refresh", true, &vec![9; 30 * MIB as usize]));
        }
        // Past it, as far as this disk goes: the heavy one holds the most, so it gives up its own.
        let mut all = store.holdings.lock().unwrap();
        assert!(make_room(&mut all, Some("bbbbbbbbbbbb"), true, 30 * MIB, 1, disk(total)));
        drop(all);
        assert_eq!(old.index()["segments"].as_array().unwrap().len(), 4, "the small one keeps its replay");
        assert!(heavy.index()["segments"][0]["n"].as_u64().unwrap() > 0, "the heavy one lost its oldest");
    }

    #[test]
    fn what_is_left_starts_at_a_whole_picture_and_a_full_disk_leaves_the_floor_free() {
        let dir = crate::private::test_dir("replays-stretch");
        let store = Store::open(&dir);
        let recording = store.create("aaaaaaaaaaaa", true).unwrap();
        recording.live.store(false, Ordering::Relaxed);
        for (i, whole) in [true, false, true, false].into_iter().enumerate() {
            let at = i as u64 * 10;
            assert!(recording.store(&store, at, at + 10, "more", whole, &vec![1; MIB as usize]));
        }
        let mut all = store.holdings.lock().unwrap();
        let held = all.get_mut("aaaaaaaaaaaa").unwrap();
        assert!(drop_oldest("aaaaaaaaaaaa", held, false));
        let left = segment_files(&held.dir);
        assert!(left[0].ends_with(".whole.bin"), "{left:?}");
        assert_eq!(left.len(), 2);
        // The last stretch is another's: it stays, unless it is the writer's own.
        assert!(!drop_oldest("aaaaaaaaaaaa", held, false));
        // Whatever the share, the floor stays free.
        let full = Disk { total: 100 * 1024 * MIB, available: FLOOR };
        assert!(
            !make_room(&mut all, Some("zzzzzzzzzzzz"), true, 2 * MIB, 90, full),
            "only a running session's last stretch is left"
        );
        drop(all);
        assert_eq!(recording.index()["segments"][0]["n"], 2, "a reader's index follows");
    }

    #[test]
    fn the_session_stopped_longest_ago_goes_whole_when_each_holds_its_last_stretch() {
        let dir = crate::private::test_dir("replays-whole");
        let store = Store::open(&dir);
        let old = fill(&store, "aaaaaaaaaaaa", 1, 20 * MIB as usize);
        std::thread::sleep(Duration::from_millis(5));
        let newer = fill(&store, "bbbbbbbbbbbb", 1, 20 * MIB as usize);
        let writer = store.create("cccccccccccc", true).unwrap();
        let mut all = store.holdings.lock().unwrap();
        // 1% of 3 GiB is some 30 MiB: the older session goes whole for the writer's bytes.
        assert!(make_room(&mut all, Some("cccccccccccc"), true, 2 * MIB, 1, disk(3 * 1024 * MIB)));
        assert!(!all.contains_key("aaaaaaaaaaaa"));
        drop(all);
        assert!(old.deleted() && !newer.deleted() && !writer.deleted());
    }

    #[test]
    fn what_can_never_fit_costs_nobody_and_a_running_session_keeps_what_its_next_frames_need() {
        let dir = crate::private::test_dir("replays-impossible");
        let store = Store::open(&dir);
        let old = fill(&store, "aaaaaaaaaaaa", 2, MIB as usize);
        let running = store.create("bbbbbbbbbbbb", true).unwrap();
        assert!(running.store(&store, 0, 10, "watch", true, &vec![2; 4 * MIB as usize]));
        let mut all = store.holdings.lock().unwrap();
        // Space for 1 MiB past the floor, and more than that coming: nothing is deleted for it.
        let tight = Disk { total: 1000 * 1024 * MIB, available: FLOOR + MIB };
        let used: u64 = all.values().map(|h| h.bytes).sum();
        assert!(!make_room(&mut all, Some("bbbbbbbbbbbb"), true, used + 2 * MIB, 90, tight));
        assert_eq!(segment_files(&all["aaaaaaaaaaaa"].dir).len(), 2);
        // Only deltas coming: its last whole picture stays, and they are what goes.
        let share = Disk { total: 500 * MIB + FLOOR, available: 500 * MIB + FLOOR };
        let need = (500 * MIB + FLOOR) / 100 - used + MIB;
        assert!(make_room(&mut all, Some("bbbbbbbbbbbb"), false, need, 1, share));
        assert_eq!(segment_files(&all["bbbbbbbbbbbb"].dir).len(), 1, "its last whole picture is there");
        drop(all);
        assert!(old.deleted(), "the room came from the session stopped longest ago");
    }

    #[test]
    fn the_index_reads_only_in_its_own_order_and_what_cannot_play_is_not_listed() {
        let dir = crate::private::test_dir("replays-index");
        let kek = crate::keys::derive("the password", &[1; crate::keys::SALT]);
        let store = Store::open(&dir);
        store.unlock(&kek);
        let recording = store.create("aaaaaaaaaaaa", true).unwrap();
        for (i, whole) in [false, true, false].into_iter().enumerate() {
            let at = i as u64 * 10;
            assert!(recording.store(&store, at, at + 10, "more", whole, b"bytes"));
        }
        recording.keep_timeline(&store, "Work", Some(1), &json!({"events": []}));
        let index = dir.join("replays/aaaaaaaaaaaa").join(INDEX);
        let bytes = std::fs::read(&index).unwrap();
        drop((recording, store));
        let reopened = |dir: &Path| Store::open(dir).unlock(&kek).remove(0).1;
        // A crash mid-rotation: what depends on a deleted stretch is not listed.
        let list = reopened(&dir).index();
        assert_eq!(list["segments"][0]["n"], 1, "{list}");
        // Lines moved around end the list where they stop reading true.
        let length = 4 + u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
        let mut moved = bytes[length..2 * length.min(bytes.len() / 2)].to_vec();
        moved.extend_from_slice(&bytes[..length]);
        std::fs::write(&index, &moved).unwrap();
        assert_eq!(reopened(&dir).index()["segments"], json!([]));
    }

    #[test]
    fn the_password_s_key_decides_whatever_public_key_stands_on_disk() {
        let dir = crate::private::test_dir("replays-vault");
        let kek = crate::keys::derive("the password", &[1; crate::keys::SALT]);
        let first = Store::open(&dir);
        first.unlock(&kek);
        let true_public = first.public.lock().unwrap().unwrap();
        // Another public key put beside the wrapped secret.
        let mut vault = std::fs::read(dir.join("replays").join(VAULT)).unwrap();
        vault[..32].copy_from_slice(&[9; 32]);
        crate::private::replace(&dir.join("replays").join(VAULT), &vault).unwrap();
        let store = Store::open(&dir);
        let running = store.create("aaaaaaaaaaaa", true).unwrap();
        store.unlock(&kek);
        assert_eq!(store.public.lock().unwrap().unwrap(), true_public);
        assert_eq!(&std::fs::read(dir.join("replays").join(VAULT)).unwrap()[..32], &true_public[..]);
        // What ran through it is sealed for the true key now, and unlocking twice makes no second pair.
        let secret = store.secret.lock().unwrap().clone().unwrap();
        let sealed = std::fs::read(running.dir.join(KEY_FILE)).unwrap();
        assert!(crate::keys::open_from(&secret, &sealed).is_some());
        assert!(store.unlock(&kek).is_empty());
    }

    #[test]
    fn an_index_written_anew_gives_its_bytes_back() {
        let dir = crate::private::test_dir("replays-compact");
        let kek = crate::keys::derive("the password", &[1; crate::keys::SALT]);
        let store = Store::open(&dir);
        store.unlock(&kek);
        let recording = store.create("aaaaaaaaaaaa", true).unwrap();
        for i in 0..1100u64 {
            assert!(recording.store(&store, i, i + 1, "refresh", true, b"x"));
        }
        recording.keep_timeline(&store, "Work", Some(1), &json!({"events": []}));
        drop((recording, store));
        for n in 0..1000u32 {
            std::fs::remove_file(dir.join("replays/aaaaaaaaaaaa").join(segment_file(n, true))).unwrap();
        }
        let store = Store::open(&dir);
        let opened = store.unlock(&kek).remove(0).1;
        let (before, _) = store.usage();
        assert_eq!(opened.index()["segments"].as_array().unwrap().len(), 100);
        store.sweep();
        assert!(store.usage().0 < before, "the index's lines are no longer charged");
        // It still reads, in its new order.
        assert_eq!(Store::open(&dir).unlock(&kek).remove(0).1.index()["segments"].as_array().unwrap().len(), 100);
    }

    #[test]
    fn a_save_after_the_last_one_changes_nothing() {
        let dir = crate::private::test_dir("replays-saves");
        let store = Store::open(&dir);
        let recording = store.create("aaaaaaaaaaaa", true).unwrap();
        recording.keep_timeline(&store, "Work", Some(5), &json!({"events": [{"seq": 1}]}));
        recording.keep_timeline(&store, "Work", None, &json!({"events": []}));
        assert_eq!(recording.timeline(0).unwrap()["events"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn archives_outlast_a_restart_and_only_the_password_opens_them() {
        let dir = crate::private::test_dir("replays-restart");
        let kek = crate::keys::derive("the password", &[1; crate::keys::SALT]);
        let store = Store::open(&dir);
        assert!(store.unlock(&kek).is_empty());
        let recording = store.create("aaaaaaaaaaaa", true).unwrap();
        assert!(recording.store(&store, 0, 10, "watch", true, b"the picture"));
        let page = json!({"events": [{"seq": 1}, {"seq": 2}], "label": "Work"});
        recording.keep_timeline(&store, "Work", Some(99), &page);
        drop((recording, store));

        let store = Store::open(&dir);
        assert_eq!(store.locked(), 1, "kept, and closed until the password");
        let other = crate::keys::derive("another password", &[1; crate::keys::SALT]);
        let kept = Store::open(&dir).unlock(&kek);
        assert_eq!(kept.len(), 1);
        let (id, recording, stopped, label) = &kept[0];
        assert_eq!((id.as_str(), *stopped, label.as_str()), ("aaaaaaaaaaaa", Some(99), "Work"));
        assert_eq!(recording.segment(0).as_deref(), Some(&b"the picture"[..]));
        assert_eq!(recording.index()["segments"][0]["whole"], true);
        assert_eq!(recording.timeline(1).unwrap()["events"].as_array().unwrap().len(), 1);
        // On disk, none of it reads: not the picture, not the label.
        for file in std::fs::read_dir(dir.join("replays/aaaaaaaaaaaa")).unwrap().flatten() {
            let bytes = std::fs::read(file.path()).unwrap();
            for said in [&b"the picture"[..], &b"Work"[..]] {
                assert!(!bytes.windows(said.len()).any(|w| w == said), "{:?}", file.path());
            }
        }
        // A password set anew opens none of them, which go.
        assert!(store.unlock(&other).is_empty());
        assert_eq!(store.locked(), 0);
        assert!(!dir.join("replays/aaaaaaaaaaaa").exists());
    }

    #[test]
    fn a_session_with_no_key_pair_yet_gets_its_key_sealed_as_the_password_comes() {
        let dir = crate::private::test_dir("replays-pending");
        let store = Store::open(&dir);
        let running = store.create("aaaaaaaaaaaa", true).unwrap();
        assert!(!running.dir.join(KEY_FILE).exists());
        store.unlock(&crate::keys::derive("the password", &[1; crate::keys::SALT]));
        assert!(running.dir.join(KEY_FILE).exists());
        // With none ever, a restart can open nothing: it goes.
        let empty = crate::private::test_dir("replays-none");
        let store = Store::open(&empty);
        store.create("bbbbbbbbbbbb", true).unwrap();
        assert_eq!(Store::open(&empty).locked(), 0);
        assert!(!empty.join("replays/bbbbbbbbbbbb").exists());
    }
}
