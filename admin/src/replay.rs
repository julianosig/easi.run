//! What the agent did on screen, recorded for the administrator's replay (see `timeline` for what it did in words).
//!
//! While the agent holds a session's control, a recorder of the admin's own watches the display as a viewer would (its
//! own encoder there, AV1 at [`FPS`] frames a second and [`BITRATE_KBPS`]) and keeps the stream as it comes: the
//! display's own messages (the configuration, lossless tiles, video, the cursor), never re-encoded, with the agent's
//! pointer, each with when it came. It records nothing while a person holds control: it stops as soon as the lease
//! leaves the agent (`State::files_lease`), before the person's input can reach the page, and drops whatever comes
//! after; it starts again, from a fresh picture, once the agent holds control again.
//!
//! The stream is cut into segments of about [`SEGMENT_SPAN`]: each starts with the configuration and the cursor as they
//! stand and asks the display for a whole picture (`KeyframeRequest`), so a replay can start at any segment. Each
//! segment is sealed (XChaCha20-Poly1305, as saved sessions are) with the recording's own key, which only the admin's
//! memory holds, into its own file under the state directory's `replays/`: a restart loses the keys, and the start
//! removes what they sealed. A recording is bounded ([`MAX_RECORDING`]), and all of them together ([`MAX_RECORDINGS`]:
//! the oldest of stopped sessions go first). Only the administrator reads them; the public demo records nothing.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use display_protocol::frame::{CodecChoice, FrameReader, Link, Settings};
use display_protocol::{CODEC_AV1, Direction, Message};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::app::{App, Session};
use crate::keys::{DataKey, Opener, PREFIX, Sealer};

/// About how long a segment lasts: a replay starts at the beginning of one.
pub const SEGMENT_SPAN: Duration = Duration::from_secs(10);
/// The most a segment holds before it is sealed anyway.
const MAX_SEGMENT: usize = 4 * 1024 * 1024;
/// The most one recording keeps on disk; past it, it stops (and says so).
pub const MAX_RECORDING: u64 = 256 * 1024 * 1024;
/// The most all recordings keep on disk together.
pub const MAX_RECORDINGS: u64 = 1024 * 1024 * 1024;
/// The recorder's stream: frames a second at most, and its video's bitrate.
const FPS: u8 = 15;
const BITRATE_KBPS: u32 = 2000;
/// How long a failed recorder waits before it watches again.
const RETRY: Duration = Duration::from_secs(2);

/// What a record of a segment is: a display message (as a viewer gets it), where the agent's pointer is (`x`, `y`,
/// big-endian `u16` each; none, both `0xffff`), or the start of a gap (control left the agent, or the recorder lost the
/// display).
pub const MESSAGE: u8 = 0;
pub const POINTER: u8 = 1;
pub const GAP: u8 = 2;

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
}

/// A session's recording.
pub struct Recording {
    /// The session's id, which each segment's seal is bound to.
    id: String,
    /// When it started: Unix milliseconds, as the timeline's events say when they happened.
    pub started: u64,
    dir: PathBuf,
    key: DataKey,
    inner: Mutex<Inner>,
    /// Recording now (its session runs); a stopped session's may be dropped for room first.
    live: AtomicBool,
}

#[derive(Default)]
struct Inner {
    segments: Vec<Segment>,
    bytes: u64,
    /// Why it stopped recording before its session did, if it did.
    stopped: Option<&'static str>,
    deleted: bool,
}

impl Recording {
    /// The recording of session `id`, under `state_dir`'s `replays/`.
    pub fn new(state_dir: &Path, id: &str) -> Result<Recording, String> {
        let dir = state_dir.join("replays").join(id);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode_0700()
            .create(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Recording {
            id: id.to_owned(),
            started: crate::timeline::now_ms(),
            dir,
            key: DataKey::random(),
            inner: Mutex::default(),
            live: AtomicBool::new(true),
        })
    }

    /// Its segments, as the API answers them: when it started, each segment, and why it stopped early, if it did.
    pub fn index(&self) -> Value {
        let inner = self.inner.lock().unwrap();
        json!({"started": self.started, "segments": inner.segments, "stopped": inner.stopped, "recording": self.live.load(Ordering::Relaxed)})
    }

    /// Segment `n`'s records, opened, if it is there.
    pub fn segment(&self, n: u32) -> Option<Vec<u8>> {
        if !self.inner.lock().unwrap().segments.iter().any(|s| s.n == n) {
            return None;
        }
        let sealed = std::fs::read(self.dir.join(format!("{n}.bin"))).ok()?;
        open(&self.key, &format!("{}/{n}", self.id), &sealed)
    }

    pub fn bytes(&self) -> u64 {
        self.inner.lock().unwrap().bytes
    }

    /// Deletes every segment for good.
    pub fn delete(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.deleted = true;
        inner.segments.clear();
        inner.bytes = 0;
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    /// Seals `plain` (segment `start`..`end`) into its file, within the bounds; false once it may record no more. The
    /// room is found and the file written under the store's lock, so recordings that write at once never pass the bound
    /// together.
    fn store(&self, store: &Store, start: u64, end: u64, starts: &'static str, plain: &[u8]) -> bool {
        let n = {
            let inner = self.inner.lock().unwrap();
            if inner.deleted || inner.stopped.is_some() {
                return false;
            }
            inner.segments.last().map_or(0, |s| s.n + 1)
        };
        let sealed = seal(&self.key, &format!("{}/{n}", self.id), plain);
        let size = sealed.len() as u64;
        if self.bytes() + size > MAX_RECORDING {
            self.inner.lock().unwrap().stopped = Some("full");
            return false;
        }
        let mut all = store.0.lock().unwrap();
        if !Store::make_room(&mut all, self, size) {
            self.inner.lock().unwrap().stopped = Some("no-room");
            return false;
        }
        let path = self.dir.join(format!("{n}.bin"));
        if let Err(e) = crate::private::replace(&path, &sealed) {
            tracing::warn!(session = %self.id, %e, "a replay segment could not be written");
            self.inner.lock().unwrap().stopped = Some("unwritten");
            return false;
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.deleted {
            let _ = std::fs::remove_file(&path);
            return false;
        }
        inner.bytes += size;
        inner.segments.push(Segment { n, start, end, size, starts });
        true
    }
}

/// Every recording, oldest first, for the bound on them all.
#[derive(Default)]
pub struct Store(Mutex<VecDeque<Arc<Recording>>>);

impl Store {
    /// Removes what earlier runs of the admin left: their keys are gone with them.
    pub fn clear(state_dir: &Path) {
        let _ = std::fs::remove_dir_all(state_dir.join("replays"));
    }

    /// Adds a new recording, forgetting those deleted since.
    pub fn add(&self, recording: Arc<Recording>) {
        let mut all = self.0.lock().unwrap();
        all.retain(|r| !r.inner.lock().unwrap().deleted);
        all.push_back(recording);
    }

    /// Whether `size` more bytes for `recording` fit within [`MAX_RECORDINGS`], once the oldest recordings of stopped
    /// sessions are deleted to make room. Called with the store's lock held, through the write it allows.
    fn make_room(all: &mut VecDeque<Arc<Recording>>, recording: &Recording, size: u64) -> bool {
        loop {
            let used: u64 = all.iter().map(|r| r.bytes()).sum();
            if used + size <= MAX_RECORDINGS {
                return true;
            }
            let Some(oldest) =
                all.iter().position(|r| !r.live.load(Ordering::Relaxed) && !std::ptr::eq(&**r, recording))
            else {
                return false;
            };
            let removed = all.remove(oldest).expect("there");
            removed.delete();
        }
    }
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
    /// How it starts (see [`Segment::starts`]).
    starts: &'static str,
}

impl Builder {
    fn new(start: u64, starts: &'static str) -> Builder {
        Builder { start, began: Instant::now(), records: Vec::new(), media: false, starts }
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
    recording: &Recording,
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
    let settings = Settings { codec: CodecChoice::Av1, bitrate_kbps: BITRATE_KBPS, max_fps: FPS };
    let hello = Message::Hello { version: display_protocol::VERSION, codecs: CODEC_AV1 };
    for link in [Link::Attach { viewer }, Link::Settings(settings), Link::Relay(hello)] {
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
                        Message::Video { .. } | Message::Tiles { .. } => {}
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
                if segment.began.elapsed() >= SEGMENT_SPAN {
                    cut = Some("refresh");
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
            let kept = keep(app, recording, done.start, end, done.starts, done.records).await;
            session.agent.timeline.touch();
            if !kept {
                return Ok(());
            }
        }
    };
    finish(app, recording, segment, at()).await?;
    session.agent.timeline.touch();
    outcome
}

/// Keeps the last segment of a watch, ended by a gap, when it holds anything of the screen.
async fn finish(app: &App, recording: &Recording, mut segment: Builder, end: u64) -> Result<(), String> {
    segment.push(end, GAP, &[]);
    if segment.media {
        keep(app, recording, segment.start, end, segment.starts, segment.records).await;
    }
    Ok(())
}

/// Seals and stores a segment off the async workers; false once the recording may not go on.
async fn keep(app: &App, recording: &Recording, start: u64, end: u64, starts: &'static str, records: Vec<u8>) -> bool {
    let (store, recording) = (app.replays.clone(), recording_handle(app, recording));
    let Some(recording) = recording else { return false };
    tokio::task::spawn_blocking(move || recording.store(&store, start, end, starts, &records)).await.unwrap_or(false)
}

/// The shared handle of `recording`, as the store keeps it.
fn recording_handle(app: &App, recording: &Recording) -> Option<Arc<Recording>> {
    app.replays.0.lock().unwrap().iter().find(|r| std::ptr::eq(&***r, recording)).cloned()
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

    #[test]
    fn a_recording_is_bounded_and_room_is_made_from_stopped_sessions_first() {
        let dir = crate::private::test_dir("replays");
        let store = Arc::new(Store::default());
        let old = Arc::new(Recording::new(&dir, "aaaaaaaaaaaa").unwrap());
        let now = Arc::new(Recording::new(&dir, "bbbbbbbbbbbb").unwrap());
        store.add(old.clone());
        store.add(now.clone());
        assert!(old.store(&store, 0, 10, "watch", b"first"));
        old.live.store(false, Ordering::Relaxed);
        assert!(now.store(&store, 0, 10, "watch", b"second"));
        assert_eq!(now.index()["segments"][0]["n"], 0);
        assert_eq!(now.segment(0).as_deref(), Some(&b"second"[..]));
        assert_eq!(now.segment(1), None);
        // Room for a segment past the bound comes from the stopped session's recording, never the live one's.
        assert!(Store::make_room(&mut store.0.lock().unwrap(), &now, MAX_RECORDINGS - now.bytes()));
        assert_eq!(old.bytes(), 0, "deleted for room");
        assert!(
            !Store::make_room(&mut store.0.lock().unwrap(), &now, MAX_RECORDINGS),
            "a live recording is never deleted for another"
        );
        now.delete();
        assert_eq!(now.segment(0), None);
    }
}
