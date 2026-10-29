//! A viewer's encoder: one thread that turns composed frames into this viewer's stream. It picks each epoch's codec,
//! sends lossless tiles for ordinary damage and for motion that tiles carry within the video's budget, video for the
//! rest of motion (AV1 only where the picture moves, see `region`; lossless tiles around it), paces itself by the
//! viewer's frame-rate cap, and reports its statistics.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use display_protocol::frame::{CodecChoice, Settings};
use display_protocol::{CODEC_AV1, CODEC_H264, Codec, MAX_VIDEO_PAYLOAD, Message, VERSION};

use super::Snapshot;
use super::av1::Av1;
use super::cursor::Pointer;
#[cfg(feature = "h264")]
use super::h264::H264;
use super::region::{self, Activity};
use super::tiles;
use super::viewer::{Mailbox, NO_REPORT, Outbox, QUEUE_BUDGET, seed_rate, send, send_media, send_media_locked};
use crate::compose::Rect;

/// Lossless-only clients get at most this many updates per second.
const TILES_ONLY_FPS: u8 = 15;
/// OpenH264's highest frame rate: it spends the bitrate over at most this many frames a second, so more would spend it
/// twice over.
const H264_MAX_FPS: u8 = 60;

/// Whether this build has the H.264 encoder (the `h264` feature).
const H264_BUILT: bool = cfg!(feature = "h264");

/// The encoder a video epoch uses.
enum Video {
    #[cfg(feature = "h264")]
    H264(Box<H264>),
    Av1(Av1),
}

impl Video {
    /// Encodes one frame, its rows `stride` pixels apart, to the codec's access unit (empty when the encoder dropped
    /// it).
    fn encode(&mut self, pixels: &[u32], stride: usize, keyframe: bool) -> Result<Vec<u8>, String> {
        match self {
            // Always the whole output.
            #[cfg(feature = "h264")]
            Video::H264(e) => e.encode(pixels, keyframe),
            Video::Av1(e) => e.encode(pixels, stride, keyframe),
        }
    }
}

/// The pixels of `region` of a frame `width` pixels wide, and how far apart its rows lie: where they lie in the
/// frame, read row by row; but the color converter misreads the last rows of a picture whose last row ends before its
/// stride does (a region on the bottom edge, off the left one), so that one is copied out into `crop`.
fn region_pixels<'a>(pixels: &'a [u32], width: usize, region: &Rect, crop: &'a mut Vec<u32>) -> (&'a [u32], usize) {
    let start = region.y as usize * width + region.x as usize;
    if pixels.len() - start >= width * region.h as usize {
        return (&pixels[start..], width);
    }
    crop.clear();
    for row in pixels[start..].chunks(width).take(region.h as usize) {
        crop.extend_from_slice(&row[..region.w as usize]);
    }
    (&crop[..], region.w as usize)
}

/// How many frames the source composed in the last [`RATE_WINDOW`] before `now` with damage on `region`, from
/// `compositions` (oldest first); `None` unless the history covers that window whole and the region's frames have come
/// without a pause over [`region::CONTINUOUS_GAP`] since before it (a region whose motion just began, beside other
/// motion or after an old repaint of the whole screen, is not measured yet).
fn source_rate(compositions: &VecDeque<(Instant, Vec<Rect>)>, region: &Rect, now: Instant) -> Option<usize> {
    let window = now.checked_sub(RATE_WINDOW)?;
    if compositions.front().is_none_or(|(t, _)| *t > window) {
        return None;
    }
    let on_region = |damage: &Vec<Rect>| damage.iter().any(|r| r.intersect(region).is_some());
    let hits: Vec<Instant> =
        compositions.iter().filter(|(t, damage)| *t <= now && on_region(damage)).map(|(t, _)| *t).collect();
    let run_start = hits
        .windows(2)
        .rev()
        .find(|pair| pair[1].duration_since(pair[0]) > region::CONTINUOUS_GAP)
        .map_or(hits.first().copied(), |pair| Some(pair[1]))?;
    let last = *hits.last()?;
    if run_start > window || now.duration_since(last) > region::CONTINUOUS_GAP {
        return None;
    }
    Some(hits.iter().filter(|t| **t > window).count())
}

fn cursor_message(epoch: u32, revision: u64, p: &Pointer) -> Message {
    let image = p.image.to_vec();
    Message::Cursor { epoch, revision, x: p.x, y: p.y, shape: p.shape, hot_x: p.hot_x, hot_y: p.hot_y, image }
}

/// The codecs a new epoch may use, best first: the person's choice when the client decodes it, then the other video
/// codec; empty means lossless tiles only. `Auto` prefers AV1 (royalty-free, better at equal bitrate).
fn codec_order(choice: CodecChoice, codecs: u8) -> Vec<Codec> {
    let order = match choice {
        CodecChoice::TilesOnly => return Vec::new(),
        CodecChoice::Auto | CodecChoice::Av1 => [Codec::Av1, Codec::H264],
        CodecChoice::H264 => [Codec::H264, Codec::Av1],
    };
    let decodes = |c: &Codec| codecs & if *c == Codec::Av1 { CODEC_AV1 } else { CODEC_H264 } != 0;
    order.into_iter().filter(|c| decodes(c) && (H264_BUILT || *c != Codec::H264)).collect()
}

/// When the encode after one that started at `started` is due. Due times advance by whole intervals, so encodes
/// start at most once per interval on average, however long each takes; one may start a quarter interval before it is
/// due, and an encoder that fell behind starts again from now instead of catching up in a burst.
fn next_due(due: Option<Instant>, started: Instant, interval: Duration) -> Instant {
    let from_now = started + interval - interval / 4;
    due.map_or(started + interval, |due| (due + interval).max(from_now))
}

/// Starts `codec`'s encoder for `width` x `height` at the settings' bitrate, spent over `fps` frames a second.
fn start_video(codec: Codec, width: u32, height: u32, settings: Settings, fps: u8) -> Result<Video, String> {
    match codec {
        Codec::Av1 => Av1::new(width, height, settings.bitrate_kbps, fps).map(Video::Av1),
        #[cfg(feature = "h264")]
        Codec::H264 => H264::new(width, height, settings.bitrate_kbps, fps).map(|e| Video::H264(Box::new(e))),
        _ => Err(format!("{codec:?} is not built into this display")),
    }
}

/// Whether a client that presented `presented` frames in a second while `sent` were sent to it fell short: under 80% of
/// at least 20 frames (fewer are too few to judge).
fn falls_short(presented: u32, sent: u32) -> bool {
    sent >= 20 && u64::from(presented) * 100 < u64::from(sent) * 80
}

/// Whether the median of encode times (microseconds) exceeds `AV1_BUDGET` of the frame interval.
fn over_budget(mut times: Vec<u32>, interval: Duration) -> bool {
    times.sort_unstable();
    times.get(times.len() / 2).is_some_and(|&t| f64::from(t) > interval.as_secs_f64() * 1e6 * AV1_BUDGET)
}

/// Records an encode that took `took` in `samples`, and once they are `count` says whether they are [`over_budget`],
/// starting the next batch.
fn sample_over_budget(took: Duration, count: usize, interval: Duration, samples: &mut Vec<u32>) -> bool {
    samples.push(took.as_micros() as u32);
    samples.len() >= count && over_budget(std::mem::take(samples), interval)
}

pub struct Encoder {
    mailbox: Arc<Mailbox>,
    out: Outbox,
    epoch: u32,
    codec: Codec,
    settings: Settings,
    video: Option<Video>,
    in_video: bool,
    sent: tiles::Sent,
    deflate: tiles::Deflate,
    /// When the next encode is due on the frame-rate cadence (see [`next_due`]).
    next_due: Option<Instant>,
    /// This viewer's revision: every encode takes the next one.
    revision: u64,
    stats: Stats,
    /// The codecs the client decodes, from its hello.
    codecs: u8,
    /// Under Auto: AV1 could not keep the frame rate in this motion episode, which goes on in H.264.
    av1_behind: bool,
    /// AV1 encode times of the current motion episode, in microseconds.
    av1_times: Vec<u32>,
    /// Video frames sent in the last full second, and client reports in a row that fell short of them.
    sent_last_second: u32,
    client_short: u8,
    /// When the current motion episode started.
    video_since: Option<Instant>,
    /// When a frame last went out as video (or the encoder started).
    last_motion: Option<Instant>,
    /// When the video encoder last failed to restart; motion goes as tiles until a retry, at most once per
    /// [`RESTART_RETRY`].
    restart_failed_at: Option<Instant>,
    /// A frame of the current motion episode did not fit the lossless budget (see [`Encoder::lossless_fits`]), so the
    /// episode goes on as video.
    lossless_over: bool,
    /// Lossless encode times of the current motion episode, in microseconds.
    lossless_times: Vec<u32>,
    /// When recent updates that the motion detector let through as lossless tiles took more than the lossless motion
    /// budget (see [`HEAVY_UPDATES`]).
    heavy: VecDeque<Instant>,
    /// Until when changes count as motion because lossless updates were costly, though the detector sees none.
    heavy_until: Option<Instant>,
    /// When each tile last changed.
    activity: Activity,
    /// Where the video encoder's frames go (the whole output for H.264, whose crops browsers would read in other
    /// colors).
    region: Option<Rect>,
    /// Since when what moves fits in a smaller region than the video's.
    smaller_since: Option<Instant>,
    /// When the video last moved to another region while it showed, and when it last sent a keyframe.
    region_moved_at: Option<Instant>,
    keyframe_at: Option<Instant>,
    /// A video frame went as tiles instead (it did not fit a message), so the next one must not refer to it.
    keyframe_owed: bool,
    /// A new epoch was configured: the client cleared its picture, so the next encode is a full refresh.
    refresh_owed: bool,
    /// A region's pixels cut from the frame, when the video encoder cannot read them in place.
    crop: Vec<u32>,
    /// Lossless tiles owed to the viewer whole: what an encode's budget left for the next ones, what a lossless trial
    /// that gave way to video recorded as sent without sending it, and the refreshes still to send.
    backlog: tiles::Backlog,
    /// The lossless bytes the current encode may send (see [`Encoder::send_owed`]): what the pacing holds.
    budget: usize,
    /// The frame rate the video encoder was started for, when it was started, and since when the video's frames have
    /// come at another rate.
    video_fps: u8,
    rate_fit_at: Option<Instant>,
    rate_off_since: Option<Instant>,
    /// When the video's frames of the last [`RATE_WINDOW`] that carried new damage of its region were encoded.
    video_sent: VecDeque<Instant>,
    /// Since when `video_sent` has counted for the current region.
    video_sent_since: Option<Instant>,
    /// When the region last shrank in this episode, and whether it has grown back soon after, so it shrinks only the
    /// slow way.
    shrunk_at: Option<Instant>,
    flapping: bool,
    /// When the last video frame's changes outside the region cost more than video would (see
    /// [`Encoder::note_outside_cost`]), and what the region must grow to take in at the next frame.
    costly_at: Option<Instant>,
    grow: Option<Rect>,
    /// What the encoder did since it last traced its flight (see [`Encoder::trace`]).
    traced: Traced,
}

/// Encodes since the last trace of the flight, by kind, and when that trace was.
#[derive(Default)]
struct Traced {
    at: Option<Instant>,
    captures: u32,
    drains: u32,
    observations: u32,
}

/// How often the encoder traces its flight, at debug level (`RUST_LOG=display::stream::encoder=debug`).
const TRACE_EVERY: Duration = Duration::from_millis(100);

/// One trace of the flight (see [`Encoder::trace`]).
struct Trace {
    in_flight: usize,
    window: usize,
    rate_kbps: u64,
    horizon_ms: u64,
    oldest_ms: u64,
    owed_cells: usize,
    in_video: bool,
    motion: bool,
    heavy: bool,
    traced: Traced,
}

impl Trace {
    fn emit(&self) {
        tracing::debug!(
            in_flight = self.in_flight,
            window = self.window,
            rate_kbps = self.rate_kbps,
            horizon_ms = self.horizon_ms,
            oldest_ms = self.oldest_ms,
            owed_cells = self.owed_cells,
            in_video = self.in_video,
            motion = self.motion,
            heavy = self.heavy,
            captures = self.traced.captures,
            drains = self.traced.drains,
            observations = self.traced.observations,
            "flight"
        );
    }
}

/// The window over which the source's frame rate is measured.
const RATE_WINDOW: Duration = Duration::from_secs(1);
/// The frame rate an encoder starts for when the source gives its region at most [`VIDEO_RATE_MAX`] frames a second, as
/// a video at 24, 25, or 30 frames a second does, and the video keeps up with it ([`KEPT_UP`]); else it starts for the
/// cap. Only these two, so a source slowed by the display's own load moves the encoder one step at most.
const VIDEO_FPS: u8 = 30;
const VIDEO_RATE_MAX: usize = 36;
/// The rate at which an encoder started for [`VIDEO_FPS`] goes back to the cap.
const CAP_RATE_MIN: usize = 45;
/// How long the video's frames must come at another rate than its encoder's before it restarts at theirs.
const RATE_OFF_FOR: Duration = Duration::from_secs(2);
/// The least time between two restarts of the encoder for the frame rate.
const RATE_REFIT_EVERY: Duration = Duration::from_secs(5);
/// The share of the source's frames the video must have encoded over the window for the source to set its rate: one
/// that falls behind (its display short of CPU, a viewer's link full) stays at the cap, since fewer frames given more
/// bits each would only fall further behind.
const KEPT_UP: f64 = 0.9;

/// Under Auto, AV1 gives way to H.264 for the rest of a motion episode when its median encode over this many frames
/// takes more than `AV1_BUDGET` of the frame interval: the stream must also leave room for composition on the same
/// CPU and for the client's decode. The next episode tries AV1 again.
const AV1_SAMPLES: usize = 20;
const AV1_BUDGET: f64 = 0.6;
/// Consecutive one-second client reports that fall short (see [`falls_short`]) before an episode leaves AV1.
const CLIENT_SHORT_SECONDS: u8 = 2;
/// How long the video encoder stays after the last video frame before it is released; a still screen then holds no
/// encoder (AV1's takes about 17 MiB at start and 44 MiB once encoding at 1280 x 800).
const VIDEO_IDLE: Duration = Duration::from_secs(5);
/// The least time between two attempts to restart a video encoder that failed to start.
const RESTART_RETRY: Duration = Duration::from_secs(1);
/// Motion goes as lossless tiles while each frame's tiles take at most this many times the video's per-frame byte
/// budget (bitrate over frame rate), and the median of their encodes over [`LOSSLESS_SAMPLES`] frames at most
/// `AV1_BUDGET` of the frame interval: one slow frame (the first of an episode warms the caches) does not decide.
const LOSSLESS_BYTES: f64 = 1.5;
const LOSSLESS_SAMPLES: usize = 5;
/// The least lossless bytes an encode may send, however little the pacing holds: one message at least goes.
const MIN_LOSSLESS: usize = 16 * 1024;
/// The share of an encode's lossless bytes new changes may take while older tiles are owed (the rest goes to them).
const FRESH_SHARE: f64 = 0.75;
/// RGB records (content too rich for a palette) a lossless motion frame may hold before its tiles are abandoned
/// without deflating the rest: libdeflate's default level compresses this much in about 3.5 ms on one core when the
/// content does not compress at all.
const LOSSLESS_RGB_BYTES: usize = 256 * 1024;
/// Updates below the motion detector's frame rate (a slideshow, a page that redraws every few hundred milliseconds)
/// count as motion once this many of them within [`HEAVY_WINDOW`] took more than the lossless motion budget (bytes or
/// encode time), and stay motion until changes stop for [`HEAVY_HOLD`].
const HEAVY_UPDATES: usize = 3;
const HEAVY_WINDOW: Duration = Duration::from_secs(2);
const HEAVY_HOLD: Duration = Duration::from_secs(1);
/// While video shows, it moves to a smaller region once what moves has fitted in one this long, and moves at most once
/// per [`REGION_MOVE_EVERY`]: each move restarts the encoder (a few to tens of milliseconds) with a keyframe, which
/// under constant bitrate looks worse for a moment. What moves in a video or a game shifts from place to place, so the
/// region grows at once but keeps what it has for a while. Changes outside the region meanwhile go as lossless tiles.
const SHRINK_AFTER: Duration = Duration::from_secs(10);
/// Two video frames whose changes outside the region cost more than [`LOSSLESS_BYTES`] times a video frame's budget,
/// this close together, grow the region over them at once.
const COSTLY_WITHIN: Duration = Duration::from_secs(2);
/// How soon it moves to a smaller region when what it would leave has not changed at all for as long.
const SHRINK_AFTER_STILL: Duration = Duration::from_secs(1);
const REGION_MOVE_EVERY: Duration = Duration::from_secs(1);
/// How long a part of the region that changed briefly must stay still before a keyframe repaints it.
const SETTLE_AFTER: Duration = Duration::from_millis(500);
/// The least time from a keyframe to a keyframe that repaints still parts of the region: under constant bitrate, a
/// keyframe soon after another gets few bits and would repaint them poorly.
const REPAINT_AFTER: Duration = Duration::from_secs(2);
/// How long into a motion episode client reports start to count.
const CLIENT_SETTLE: Duration = Duration::from_secs(2);
/// How long an idle encoder sleeps before it looks at its mailbox again (a new frame wakes it at once).
const IDLE_WAIT: Duration = Duration::from_millis(100);
/// Encode times kept for the statistics' percentiles.
const ENCODE_SAMPLES: usize = 512;

#[derive(Default)]
struct Stats {
    since: Option<Instant>,
    frames: u32,
    /// Video frames sent, which the client's reports of video frames drawn are compared with.
    video_frames: u32,
    bytes: u32,
    encode_us: VecDeque<u32>,
}

/// The revision of the last of `messages`, if any.
fn last_revision(messages: &[Message]) -> Option<u64> {
    messages.iter().rev().find_map(|m| if let Message::Tiles { revision, .. } = m { Some(*revision) } else { None })
}

impl Encoder {
    pub fn new(mailbox: Arc<Mailbox>, out: Outbox) -> Encoder {
        Encoder {
            sent: tiles::Sent::new(mailbox.width, mailbox.height),
            deflate: tiles::compressor(),
            out,
            epoch: 0,
            codec: Codec::None,
            settings: Settings::default(),
            video: None,
            in_video: false,
            next_due: None,
            revision: 0,
            stats: Stats::default(),
            codecs: 0,
            av1_behind: false,
            av1_times: Vec::new(),
            sent_last_second: 0,
            client_short: 0,
            video_since: None,
            last_motion: None,
            restart_failed_at: None,
            lossless_over: false,
            lossless_times: Vec::new(),
            heavy: VecDeque::new(),
            heavy_until: None,
            activity: Activity::new(mailbox.width, mailbox.height),
            region: None,
            smaller_since: None,
            region_moved_at: None,
            keyframe_at: None,
            keyframe_owed: false,
            refresh_owed: false,
            crop: Vec::new(),
            backlog: tiles::Backlog::new(mailbox.width, mailbox.height),
            budget: usize::MAX,
            video_fps: 0,
            rate_fit_at: None,
            rate_off_since: None,
            video_sent: VecDeque::new(),
            video_sent_since: None,
            shrunk_at: None,
            flapping: false,
            costly_at: None,
            grow: None,
            traced: Traced::default(),
            mailbox,
        }
    }

    /// Whether the video encoder has gone unused long enough to release (it holds tens of MiB; see `VIDEO_IDLE`).
    fn video_idle(&self) -> bool {
        self.video.is_some() && !self.in_video && self.last_motion.is_some_and(|t| t.elapsed() >= VIDEO_IDLE)
    }

    /// Whether an AV1 episode has an H.264 encoder to continue in: this build has one and the client decodes it.
    fn can_fall_back(&self) -> bool {
        H264_BUILT && self.codecs & CODEC_H264 != 0
    }

    /// How early in its period an encode may start: a frame that arrives a little early is not held back a whole
    /// interval.
    fn slack(&self) -> Duration {
        self.frame_interval() / 4
    }

    fn frame_interval(&self) -> Duration {
        let fps =
            if self.codec == Codec::None { self.settings.max_fps.min(TILES_ONLY_FPS) } else { self.video_max_fps() };
        Duration::from_secs_f64(1.0 / f64::from(fps.max(1)))
    }

    /// The settings' frame-rate cap for video, as this epoch's codec takes it: H.264 at most [`H264_MAX_FPS`].
    fn video_max_fps(&self) -> u8 {
        let cap = if self.codec == Codec::H264 { H264_MAX_FPS } else { u8::MAX };
        self.settings.max_fps.min(cap).max(1)
    }

    pub fn run(mut self) {
        loop {
            let work = {
                let mailbox = self.mailbox.clone();
                let mut pending = mailbox.pending.lock().unwrap();
                let mut drop_video = false;
                let mut observe = None;
                let mut drain = None;
                loop {
                    if pending.closed {
                        return;
                    }
                    // Media in flight holds capture back while it is too old or too large for the connection, and the
                    // pacing while what went out last is in debt; the pointer too (a custom one carries its image).
                    // Both take one turn on one decision, so a pointer the person keeps moving never takes every turn
                    // from the picture; meanwhile the latest pointer wins.
                    let seed = seed_rate(&pending.settings);
                    let motion = pending.latest.as_ref().is_some_and(|s| s.motion_at(Instant::now()));
                    // Logged without the mailbox's lock, which the compositor and the reader also take.
                    if let Some(trace) = self.trace(&pending.flight, seed, motion) {
                        drop(pending);
                        trace.emit();
                        pending = mailbox.pending.lock().unwrap();
                        // The next trace waits its interval from now, so a slow log never leaves no turn for the work.
                        self.traced.at = Some(Instant::now());
                        continue;
                    }
                    let pacing = pending.flight.pacing(Instant::now(), seed);
                    let room = self.mailbox.queued.load(Ordering::Relaxed) < QUEUE_BUDGET
                        && pacing.is_zero()
                        && pending.flight.admits(Instant::now(), seed);
                    if pending.pointer_dirty && self.epoch > 0 && room {
                        pending.pointer_dirty = false;
                        self.revision += 1;
                        let pointer = pending.pointer.as_ref().expect("a dirty pointer exists");
                        let message = cursor_message(self.epoch, self.revision, pointer);
                        if !send_media_locked(&self.out, &self.mailbox, &mut pending, &message) {
                            return;
                        }
                    }
                    let ready = pending.hello.is_some() && pending.latest.is_some();
                    let due = self.next_due.is_none_or(|at| Instant::now() + self.slack() >= at);
                    let changed =
                        self.refresh_owed || !pending.dirty.is_empty() || pending.refresh || pending.settings_changed;
                    let now = Instant::now();
                    let motion_ends = self.video_may_end(pending.latest.as_deref(), now);
                    // What an encode may send at once: what the pacing holds.
                    self.budget = pending.flight.credit(now, seed).max(MIN_LOSSLESS);
                    if room && ready && due && (changed || motion_ends) {
                        break;
                    }
                    // Between captures, what is still owed goes on as the pacing allows (not in a new epoch's place).
                    let fresh_epoch = self.refresh_owed || pending.refresh || pending.settings_changed;
                    if room && ready && self.epoch > 0 && !self.backlog.is_empty() && !fresh_epoch {
                        drain = pending.latest.clone();
                        break;
                    }
                    if self.video_idle() {
                        drop_video = true;
                        break;
                    }
                    // Held back by the link, it still looks at each new picture (see `observe`).
                    if !room && ready && !pending.unobserved.is_empty() {
                        let damage = pending.unobserved.take();
                        observe = pending.latest.clone().map(|snapshot| (snapshot, damage));
                        break;
                    }
                    let wait = if ready && room && !due {
                        self.next_due
                            .map_or(Duration::ZERO, |at| at.saturating_duration_since(Instant::now() + self.slack()))
                    } else if !pacing.is_zero() {
                        pacing.min(IDLE_WAIT)
                    } else {
                        IDLE_WAIT
                    };
                    pending = mailbox.wake.wait_timeout(pending, wait).unwrap().0;
                }
                if let Some((snapshot, damage)) = observe {
                    drop(pending);
                    self.traced.observations += 1;
                    self.observe(&snapshot, &damage, Instant::now());
                    continue;
                }
                if let Some(snapshot) = drain {
                    drop(pending);
                    self.traced.drains += 1;
                    if !self.drain(&snapshot) {
                        self.mailbox.close();
                        return;
                    }
                    continue;
                }
                if drop_video {
                    None
                } else {
                    let snapshot = pending.latest.clone().expect("checked above");
                    // The encode looks at all the damage since the last one.
                    pending.unobserved.take();
                    let codecs = pending.hello.expect("checked above");
                    // The flag is taken first: settings that came before the hello (the admin sends its own right
                    // after attaching) belong to the first epoch, and must not open a second one.
                    let reconfigure = std::mem::take(&mut pending.settings_changed) || self.epoch == 0;
                    let refresh = std::mem::take(&mut pending.refresh);
                    let dirty = pending.dirty.take();
                    let settings = pending.settings;
                    Some((snapshot, codecs, reconfigure, refresh, dirty, settings))
                }
            };
            // The video encoder is released outside the mailbox lock: closing one drains its stream.
            let Some((snapshot, codecs, reconfigure, refresh, dirty, settings)) = work else {
                self.video = None;
                self.region = None;
                super::trim_heap();
                continue;
            };
            if reconfigure && !self.configure(codecs, settings) {
                self.mailbox.close();
                return;
            }
            self.traced.captures += 1;
            if !self.encode(&snapshot, refresh, dirty) {
                self.mailbox.close();
                return;
            }
        }
    }

    /// Picks the codec and starts a new epoch with its configuration, which the next encode opens with a full refresh.
    fn configure(&mut self, codecs: u8, settings: Settings) -> bool {
        self.settings = settings;
        self.refresh_owed = true;
        self.codecs = codecs;
        self.av1_times.clear();
        self.client_short = 0;
        self.epoch += 1;
        self.mailbox.epoch.store(self.epoch, Ordering::Relaxed);
        self.in_video = false;
        self.video = None;
        self.region = None;
        // The video encoder starts with the first frame that goes as video (see `encode`).
        // Leaving AV1 behind belongs to Auto alone: under another choice there is nothing to come back to.
        self.av1_behind &= settings.codec == CodecChoice::Auto;
        self.codec = codec_order(settings.codec, codecs)
            .into_iter()
            .find(|&c| !(self.av1_behind && c == Codec::Av1))
            .unwrap_or(Codec::None);
        let sent = send(
            &self.out,
            &self.mailbox,
            &Message::Config {
                version: VERSION,
                epoch: self.epoch,
                width: self.mailbox.width as u16,
                height: self.mailbox.height as u16,
                codec: self.codec,
                max_fps: settings.max_fps,
            },
        );
        self.mailbox.configured.store(true, Ordering::Relaxed);
        // A new configuration resets the client's cursor; the current pointer follows at this epoch.
        let mut pending = self.mailbox.pending.lock().unwrap();
        pending.pointer_dirty |= pending.pointer.is_some();
        sent
    }

    fn encode(&mut self, snapshot: &Snapshot, refresh: bool, dirty: Vec<Rect>) -> bool {
        let started = Instant::now();
        self.activity.observe(&snapshot.pixels, &dirty, started);
        // The source's motion, which ends an episode; costly updates the detector missed count too while they go on.
        let heavy = self.heavy_at(started);
        if heavy && !dirty.is_empty() {
            self.heavy_until = Some(started + HEAVY_HOLD);
        }
        // Video ends once no pixel changed for a while, whatever the damage says (a page may repaint the same pixels).
        let still = self.in_video && self.activity.region(started).is_none();
        let source_motion = (snapshot.motion_at(started) || heavy) && self.codec != Codec::None && !still;
        // A motion episode is over: its timing samples go with it, and after one that left AV1 behind the next starts
        // in AV1 again.
        if !source_motion {
            self.lossless_over = false;
            self.lossless_times.clear();
            self.av1_times.clear();
            if self.av1_behind {
                self.av1_behind = false;
                if !self.configure(self.codecs, self.settings) {
                    return false;
                }
            }
        }
        // Every epoch begins with a full refresh, lossless tiles or, while video goes on, a keyframe and the tiles
        // around it: a client asks only once it loses decodability.
        let refresh = std::mem::take(&mut self.refresh_owed) || refresh;
        self.revision += 1;
        // Motion tries lossless tiles first, until a frame does not fit the budget.
        let video_dirty = dirty.clone();
        if !source_motion || (!self.lossless_over && !self.in_video) {
            // Owed whole: the whole output on a refresh (each epoch's first, or the client's request), and the video's
            // region when video ends, at newer revisions than its frames (around the region, tiles kept the picture
            // exact, and what a region left as it moved was owed then).
            let output = Rect::new(0, 0, self.mailbox.width, self.mailbox.height);
            if refresh {
                self.backlog.clear();
                self.backlog.push(region::tiles_outside(&output, None));
            } else if self.in_video {
                self.backlog.push(region::tiles_outside(&self.region.unwrap_or(output), None));
            }
            let owed = self.owed(&snapshot.pixels, None);
            let fresh = self.sent.changed(&snapshot.pixels, dirty);
            if !source_motion {
                let (fresh_bytes, bytes, ok) = self.send_owed(&snapshot.pixels, fresh, owed);
                if self.codec != Codec::None {
                    self.note_update(started, fresh_bytes, started.elapsed());
                }
                self.in_video = false;
                return self.finish(started, bytes, ok);
            }
            // Motion stays lossless while the whole update fits the video's budget; it is sent within this encode's.
            let all: Vec<Rect> = fresh.iter().chain(&owed).copied().collect();
            let width = self.mailbox.width;
            let trial = tiles::encode(
                &mut self.deflate,
                &snapshot.pixels,
                width,
                &all,
                self.epoch,
                self.revision,
                LOSSLESS_RGB_BYTES,
                usize::MAX,
            );
            let bytes: usize = trial.iter().flat_map(|(m, _)| m).map(tiles::wire_bytes).sum();
            if let Some((messages, _)) = trial.filter(|_| self.lossless_fits(bytes, started.elapsed())) {
                self.in_video = false;
                let (bytes, ok) = self.send_within(messages);
                return self.finish(started, bytes, ok);
            }
            // Recorded as sent, the tiles stay owed: the video that replaces them covers them where it lies, they go
            // as tiles elsewhere (they count as changed now), and it ends with a refresh of its region.
            tracing::debug!(bytes, "motion leaves lossless tiles for video");
            self.activity.mark(&fresh, started);
            self.backlog.push(owed);
            self.backlog.push(fresh);
            self.lossless_over = true;
        }
        self.encode_video(snapshot, refresh, started, video_dirty)
    }

    /// Takes what is owed (but what lies inside `region`, which video covers), recorded as sent with `pixels`: they
    /// go whole as they are now.
    fn owed(&mut self, pixels: &[u32], region: Option<&Rect>) -> Vec<Rect> {
        if let Some(region) = region {
            self.backlog.retain(|r| !region.contains_rect(r));
        }
        let owed = self.backlog.take();
        self.sent.record(pixels, &owed);
        owed
    }

    /// Sends `fresh` (new changes) and then `owed` (older tiles, recorded) as lossless tiles of `pixels`, at most
    /// about this encode's budget: the fresh take at most [`FRESH_SHARE`] of it while any are owed, so none waits for
    /// ever, and what does not fit stays owed, the older first. Returns the bytes of fresh tiles sent, the bytes sent,
    /// and whether every message was queued; the revision becomes the last message's.
    fn send_owed(&mut self, pixels: &[u32], fresh: Vec<Rect>, owed: Vec<Rect>) -> (usize, usize, bool) {
        let share = if owed.is_empty() { self.budget } else { (self.budget as f64 * FRESH_SHARE) as usize };
        let width = self.mailbox.width;
        let (first, sent_fresh) = tiles::encode(
            &mut self.deflate,
            pixels,
            width,
            &fresh,
            self.epoch,
            self.revision,
            usize::MAX,
            share.max(1),
        )
        .expect("an encode without an RGB limit completes");
        let fresh_bytes: usize = first.iter().map(tiles::wire_bytes).sum();
        let next = last_revision(&first).map_or(self.revision, |r| r + 1);
        let left = self.budget.saturating_sub(fresh_bytes).max(1);
        let (second, sent_owed) =
            tiles::encode(&mut self.deflate, pixels, width, &owed, self.epoch, next, usize::MAX, left)
                .expect("an encode without an RGB limit completes");
        self.backlog.push(owed[sent_owed..].iter().copied());
        self.backlog.push(fresh[sent_fresh..].iter().copied());
        let (bytes_first, ok_first) = self.send_tiles(&first);
        let (bytes_second, ok_second) = self.send_tiles(&second);
        (fresh_bytes, bytes_first + bytes_second, ok_first & ok_second)
    }

    /// Sends `messages` (from this revision on) as far as this encode's budget goes, at least one; the rectangles of the
    /// others stay owed. Returns the bytes sent and whether every message was queued; the revision becomes the last
    /// sent message's.
    fn send_within(&mut self, messages: Vec<Message>) -> (usize, bool) {
        let mut spent = 0;
        let fits = messages
            .iter()
            .take_while(|m| {
                let fits = spent == 0 || spent < self.budget;
                spent += tiles::wire_bytes(m);
                fits
            })
            .count();
        let (now, later) = messages.split_at(fits);
        for message in later {
            if let Message::Tiles { rects, .. } = message {
                let rects = rects.iter().map(|r| Rect::new(r.x.into(), r.y.into(), r.w.into(), r.h.into()));
                self.backlog.push(rects);
            }
        }
        self.send_tiles(now)
    }

    /// Sends what is still owed, within this encode's budget, from `snapshot` (the latest picture) without capturing
    /// anything: between captures, a refresh larger than one encode's budget goes on at the pacing's rate rather than
    /// the frame rate's. What lies inside the video's region is no longer owed: the video covers it.
    fn drain(&mut self, snapshot: &Snapshot) -> bool {
        let started = Instant::now();
        let region = self.region.filter(|_| self.in_video);
        let owed = self.owed(&snapshot.pixels, region.as_ref());
        if owed.is_empty() {
            return true;
        }
        self.revision += 1;
        let (_, bytes, ok) = self.send_owed(&snapshot.pixels, Vec::new(), owed);
        self.record(started.elapsed(), bytes);
        ok
    }

    /// What to trace, at most every [`TRACE_EVERY`] and at debug level: what is in flight and owed, and what the
    /// encoder did (nothing while idle).
    fn trace(&mut self, flight: &super::flight::Flight, seed: f64, motion: bool) -> Option<Trace> {
        let now = Instant::now();
        if !tracing::enabled!(tracing::Level::DEBUG) || self.traced.at.is_some_and(|t| now - t < TRACE_EVERY) {
            return None;
        }
        let traced = std::mem::take(&mut self.traced);
        self.traced.at = Some(now);
        if flight.bytes() == 0 && self.backlog.is_empty() && traced.captures + traced.drains == 0 {
            return None;
        }
        Some(Trace {
            in_flight: flight.bytes(),
            window: flight.window(now, seed),
            rate_kbps: (flight.delivery_rate(now, seed) * 8.0 / 1000.0) as u64,
            horizon_ms: flight.horizon(now).as_millis() as u64,
            oldest_ms: flight.oldest_age(now).as_millis() as u64,
            owed_cells: self.backlog.len(),
            in_video: self.in_video,
            motion,
            heavy: self.heavy_at(now),
            traced,
        })
    }

    /// Observes, at `now`, what `damage` changed of `snapshot` without encoding it: while the viewer's link holds
    /// capture back, the encoder still watches the picture at the compositor's pace, so what moves for good is seen to
    /// (and the video's region follows it) however seldom it encodes.
    fn observe(&mut self, snapshot: &Snapshot, damage: &[Rect], now: Instant) {
        self.activity.observe(&snapshot.pixels, damage, now);
    }

    /// Whether video showing may end at `now`, with `latest` the latest picture: once the source's motion has (and no
    /// costly updates hold it), or once no pixel has changed for [`region::ACTIVE_WINDOW`] since one changed in this
    /// episode, whatever holds it, so the exact refresh of its region starts as soon as the picture is still (an
    /// episode the encoder has seen no change in yet, as one a viewer joins, is not taken for a still one).
    fn video_may_end(&self, latest: Option<&Snapshot>, now: Instant) -> bool {
        let stilled = self.activity.last_change().is_some_and(|last| {
            self.video_since.is_some_and(|since| last >= since)
                && now.saturating_duration_since(last) >= region::ACTIVE_WINDOW
        });
        self.in_video && ((!self.heavy_at(now) && latest.is_some_and(|s| !s.motion_at(now))) || stilled)
    }

    /// Whether costly lossless updates make changes count as motion at `now`.
    fn heavy_at(&self, now: Instant) -> bool {
        self.heavy_until.is_some_and(|t| now < t)
    }

    /// Records a lossless update outside motion of `bytes` that started at `started` and took `took`; enough costly
    /// ones (over the lossless motion budget in bytes or in time) start a motion episode.
    fn note_update(&mut self, started: Instant, bytes: usize, took: Duration) {
        let slow = took.as_secs_f64() > self.frame_interval().as_secs_f64() * AV1_BUDGET;
        if (bytes as f64) <= self.frame_bytes() * LOSSLESS_BYTES && !slow {
            return;
        }
        self.heavy.retain(|t| started.duration_since(*t) < HEAVY_WINDOW);
        self.heavy.push_back(started);
        if self.heavy.len() >= HEAVY_UPDATES {
            self.heavy.clear();
            self.heavy_until = Some(started + HEAVY_HOLD);
        }
    }

    /// Records a lossless motion frame of `bytes` that took `took` and says whether motion keeps within the video's
    /// budget.
    fn lossless_fits(&mut self, bytes: usize, took: Duration) -> bool {
        if bytes as f64 > self.frame_bytes() * LOSSLESS_BYTES {
            return false;
        }
        !sample_over_budget(took, LOSSLESS_SAMPLES, self.frame_interval(), &mut self.lossless_times)
    }

    /// The video's bytes per frame: its bitrate over its frame rate.
    fn frame_bytes(&self) -> f64 {
        seed_rate(&self.settings) * self.frame_interval().as_secs_f64()
    }

    /// Encodes `rects` of `pixels` as lossless tiles at this epoch, from this revision on (one per message), whatever
    /// their size, and queues them: their bytes, and whether every one was queued. The revision becomes the last
    /// message's.
    fn send_lossless(&mut self, pixels: &[u32], rects: &[Rect]) -> (usize, bool) {
        let width = self.mailbox.width;
        let (messages, _) =
            tiles::encode(&mut self.deflate, pixels, width, rects, self.epoch, self.revision, usize::MAX, usize::MAX)
                .expect("an unbounded encode always completes");
        self.send_tiles(&messages)
    }

    /// Queues tiles messages for the viewer: their bytes, and whether every one was queued. The revision becomes the
    /// last message's.
    fn send_tiles(&mut self, messages: &[Message]) -> (usize, bool) {
        self.revision = last_revision(messages).unwrap_or(self.revision);
        messages.iter().fold((0, true), |(bytes, ok), message| {
            (bytes + tiles::wire_bytes(message), send_media(&self.out, &self.mailbox, message) & ok)
        })
    }

    fn finish(&mut self, started: Instant, bytes: usize, ok: bool) -> bool {
        self.next_due = Some(next_due(self.next_due, started, self.frame_interval()));
        self.record(started.elapsed(), bytes);
        ok
    }

    /// The region the video should take at `started`, if it should move: at the start of an episode, where things move
    /// for good, else where anything changed (anywhere, if the detector saw motion the tiles do not show); while it
    /// shows, a region that holds what moves for good outside the current one, or a smaller one once that has fitted in
    /// it for [`SHRINK_AFTER`] ([`SHRINK_AFTER_STILL`] when what it would leave is wholly still), at most once per
    /// [`REGION_MOVE_EVERY`]; a region that grew back within [`SHRINK_AFTER`] of shrinking (an overlay that comes and
    /// goes at its edge) shrinks only the slow way for the rest of the episode. Brief changes outside the region (a
    /// hover, a ticking counter) go as tiles and never move it.
    fn next_region(&mut self, started: Instant) -> Option<Rect> {
        let output = Rect::new(0, 0, self.mailbox.width, self.mailbox.height);
        let wanted = if self.codec == Codec::Av1 { self.activity.moving(started) } else { Some(output) };
        let current = self.region.filter(|_| self.video.is_some());
        let Some(current) = current.filter(|_| self.in_video) else {
            // An episode's first frames: what moves for good, else whatever changed.
            (self.shrunk_at, self.flapping, self.costly_at, self.grow) = (None, false, None, None);
            let wanted = wanted.or_else(|| self.activity.region(started)).unwrap_or(output);
            return (current != Some(wanted)).then_some(wanted);
        };
        // What changed around the video at a greater cost than video, frame after frame, moves with it (a video gone
        // fullscreen): the region takes it in at once.
        if let Some(grow) = self.grow.take().map(|grow| current.union(&grow)).filter(|g| *g != current) {
            self.smaller_since = None;
            self.flapping |= self.shrunk_at.is_some_and(|t| started.duration_since(t) < SHRINK_AFTER);
            return Some(grow);
        }
        let refit = self.rate_refit(started, &current);
        let Some(wanted) = wanted.filter(|w| *w != current) else {
            self.smaller_since = None;
            return refit.then_some(current);
        };
        let rested = self.region_moved_at.is_none_or(|t| started.duration_since(t) >= REGION_MOVE_EVERY);
        if !current.contains_rect(&wanted) {
            self.smaller_since = None;
            if !rested {
                return refit.then_some(current);
            }
            self.flapping |= self.shrunk_at.is_some_and(|t| started.duration_since(t) < SHRINK_AFTER);
            return Some(wanted);
        }
        let since = *self.smaller_since.get_or_insert(started);
        // What the region would leave: wholly still (a page that changed around a video), it goes back to exact tiles
        // soon; still changing a little (the rest of a video or a game), it stays video longer.
        let still = !self.flapping && self.activity.still_outside(&current, &wanted, SHRINK_AFTER_STILL, started);
        let after = if still { SHRINK_AFTER_STILL } else { SHRINK_AFTER };
        if rested && started.duration_since(since) >= after {
            self.shrunk_at = Some(started);
            return Some(wanted);
        }
        refit.then_some(current)
    }

    /// The frame rate an encoder of `region` should start for at `now`: [`VIDEO_FPS`] or the cap, by the rate the
    /// source gives the region its frames at; `None` while that is not known.
    fn region_fps(&self, now: Instant, region: &Rect) -> Option<u8> {
        let cap = self.video_max_fps();
        let rate = source_rate(&self.mailbox.pending.lock().unwrap().compositions, region, now)?;
        let window = now.checked_sub(RATE_WINDOW)?;
        if self.video_sent_since.is_none_or(|since| since > window) {
            return None;
        }
        let sent = self.video_sent.iter().filter(|t| **t > window && **t <= now).count();
        let kept_up = sent as f64 >= rate as f64 * KEPT_UP;
        // Between the two thresholds the encoder keeps the rate it has, so a source near one does not flap.
        let video = rate <= VIDEO_RATE_MAX || (rate < CAP_RATE_MIN && self.video_fps == VIDEO_FPS.min(cap));
        Some(if video && kept_up { VIDEO_FPS.min(cap) } else { cap })
    }

    /// The frame rate an encoder of `next` starts for at `now`, after one of `from` (while video `showing`): the
    /// source's rate when known, else what the encoder had, or the cap at an episode's start. What was sent counts only
    /// for a region within the one it covered; until a new region has a window of its own, the encoder keeps its rate.
    fn start_fps(&mut self, now: Instant, next: &Rect, from: Option<Rect>, showing: bool) -> u8 {
        if from.is_none_or(|from| !from.contains_rect(next)) {
            self.video_sent.clear();
            self.video_sent_since = None;
        }
        let fitted = Some(self.video_fps).filter(|fps| *fps > 0 && showing);
        self.region_fps(now, next).or(fitted).unwrap_or(self.video_max_fps())
    }

    /// Records that a video frame was encoded at `now`.
    fn note_video_frame(&mut self, now: Instant) {
        self.video_sent_since.get_or_insert(now);
        self.video_sent.push_back(now);
        while self.video_sent.front().is_some_and(|t| now.duration_since(*t) > RATE_WINDOW) {
            self.video_sent.pop_front();
        }
    }

    /// Whether the video's encoder should restart for the rate the source gives its region frames at: another rate
    /// for [`RATE_OFF_FOR`], at most once per [`RATE_REFIT_EVERY`].
    fn rate_refit(&mut self, now: Instant, region: &Rect) -> bool {
        let off = self.region_fps(now, region).is_some_and(|fps| fps != self.video_fps);
        if !off {
            self.rate_off_since = None;
            return false;
        }
        let since = *self.rate_off_since.get_or_insert(now);
        now.duration_since(since) >= RATE_OFF_FOR
            && self.rate_fit_at.is_none_or(|t| now.duration_since(t) >= RATE_REFIT_EVERY)
    }

    fn encode_video(&mut self, snapshot: &Snapshot, refresh: bool, started: Instant, dirty: Vec<Rect>) -> bool {
        let showing = self.in_video;
        let moved_from = self.region.filter(|_| showing && self.video.is_some());
        let may_retry = self.restart_failed_at.is_none_or(|t| started.duration_since(t) >= RESTART_RETRY);
        let mut moved = false;
        if let Some(next) = self.next_region(started).filter(|_| may_retry) {
            // The encoder starts with an episode's first video frame (with a keyframe), restarts when the region
            // moves, and is released once still.
            self.video = None;
            self.region = None;
            let fps = self.start_fps(started, &next, moved_from, showing);
            match start_video(self.codec, next.w as u32, next.h as u32, self.settings, fps) {
                Ok(video) => {
                    self.video = Some(video);
                    (self.video_fps, self.rate_fit_at, self.rate_off_since) = (fps, Some(started), None);
                    self.region = Some(next);
                    moved = true;
                    self.smaller_since = None;
                    if showing {
                        self.region_moved_at = Some(started);
                    }
                }
                Err(e) => {
                    tracing::error!(%e, codec = ?self.codec, "video encoder failed to start; motion goes as tiles for now");
                    self.restart_failed_at = Some(started);
                }
            }
        }
        let Some(region) = self.region.filter(|_| self.video.is_some()) else {
            // Without an encoder, motion goes as a full lossless refresh, within the budget like any other.
            self.backlog.push(region::tiles_outside(&Rect::new(0, 0, self.mailbox.width, self.mailbox.height), None));
            let owed = self.owed(&snapshot.pixels, None);
            self.in_video = false;
            let (_, bytes, ok) = self.send_owed(&snapshot.pixels, Vec::new(), owed);
            return self.finish(started, bytes, ok);
        };
        self.last_motion = Some(started);
        // Inside the region, a part that changed and then stayed still may keep traces of what it showed before (the
        // encoder codes only what it deems visible): a keyframe repaints it, no sooner than REPAINT_AFTER the last one.
        let repaint = self.keyframe_at.is_some_and(|t| {
            started.duration_since(t) >= REPAINT_AFTER && self.activity.settled(&region, t, SETTLE_AFTER, started)
        });
        let keyframe = refresh || !showing || moved || repaint || self.keyframe_owed;
        let inside = dirty.iter().any(|r| r.intersect(&region).is_some());
        // Around the video, lossless tiles: what changed there, all of it on a refresh, and, owed whole, whatever the
        // video left when it moved (its old picture there, whatever the client was sent before) and whatever went
        // unsent around it (a lossless trial's, an earlier encode's beyond its budget). They take this revision on and
        // the video the next, so a late frame of the old region never covers them. What is owed inside the region,
        // the video covers.
        if refresh {
            self.backlog.clear();
            self.backlog.push(region::tiles_outside(&Rect::new(0, 0, self.mailbox.width, self.mailbox.height), None));
        } else if let Some(old) = moved_from.filter(|_| moved) {
            self.backlog.push(region::tiles_outside(&old, Some(&region)));
        }
        let owed = self.owed(&snapshot.pixels, Some(&region));
        let around: Vec<Rect> = dirty.into_iter().filter(|r| !region.contains_rect(r)).collect();
        let fresh = if refresh { Vec::new() } else { self.sent.changed(&snapshot.pixels, around) };
        let mut video_took = None;
        let (fresh_bytes, mut bytes, mut ok) = self.send_owed(&snapshot.pixels, fresh.clone(), owed);
        self.note_outside_cost(started, showing, fresh_bytes, &fresh);
        self.revision += u64::from(bytes > 0);
        if keyframe || inside {
            let (pixels, stride) =
                region_pixels(&snapshot.pixels, self.mailbox.width as usize, &region, &mut self.crop);
            let encode_started = Instant::now();
            let encoded = self.video.as_mut().expect("checked above").encode(pixels, stride, keyframe);
            video_took = Some(encode_started.elapsed());
            // Only frames with new damage of the region keep up with the source; a refresh or a keyframe alone does
            // not.
            if inside {
                self.note_video_frame(started);
            }
            if keyframe {
                self.keyframe_at = Some(started);
            }
            match encoded {
                Ok(payload) if payload.len() > MAX_VIDEO_PAYLOAD => {
                    // Too large for one message: the region goes as lossless tiles, which split under the bound, at
                    // once whatever their size (only a frame past 2 MiB gets here, and the next is a keyframe of the
                    // same region, which would drop them if they were owed).
                    tracing::debug!(bytes = payload.len(), "a video frame over the message bound goes as tiles");
                    let rects = region::tiles_outside(&region, None);
                    self.sent.record(&snapshot.pixels, &rects);
                    let (tile_bytes, tiles_ok) = self.send_lossless(&snapshot.pixels, &rects);
                    (bytes, ok) = (bytes + tile_bytes, ok & tiles_ok);
                    self.keyframe_owed = true;
                }
                Ok(payload) if !payload.is_empty() => {
                    self.keyframe_owed &= !keyframe;
                    bytes += payload.len();
                    self.stats.video_frames += 1;
                    ok &= send_media(
                        &self.out,
                        &self.mailbox,
                        &Message::Video {
                            epoch: self.epoch,
                            revision: self.revision,
                            rect: region.into(),
                            keyframe,
                            timestamp_us: snapshot.time_us,
                            payload,
                        },
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::error!(%e, "video encoding failed");
                    return false;
                }
            }
        }
        if !self.in_video {
            // A new episode: client reports from before it say nothing about this stream.
            self.video_since = Some(started);
            self.client_short = 0;
            self.mailbox.client_presented.store(NO_REPORT, Ordering::Relaxed);
        }
        self.in_video = true;
        let auto_av1 = self.codec == Codec::Av1 && self.settings.codec == CodecChoice::Auto;
        let behind = video_took.is_some_and(|took| self.av1_falls_behind(took));
        if auto_av1 && (behind || self.client_falls_behind()) {
            tracing::info!(
                "AV1 cannot keep the frame rate here or at the client; this motion episode goes on in H.264"
            );
            self.av1_behind = true;
            if !self.configure(self.codecs, self.settings) {
                return false;
            }
        }
        self.finish(started, bytes, ok)
    }

    /// Records what changed outside the video's region in a frame of the episode it was `showing`, `bytes` as lossless
    /// tiles: costlier than [`LOSSLESS_BYTES`] times a video frame's budget in two frames in a row, at most
    /// [`COSTLY_WITHIN`] apart, it moves like the video (a video gone fullscreen, which as tiles would cost the link
    /// several times what video does and slow everything down), and the next frame's region takes it in. A one-off
    /// change beside a playing video (a new page around it) stays exact.
    fn note_outside_cost(&mut self, started: Instant, showing: bool, bytes: usize, rects: &[Rect]) {
        if !showing || bytes as f64 <= self.frame_bytes() * LOSSLESS_BYTES {
            self.costly_at = None;
            return;
        }
        if self.costly_at.is_some_and(|t| started.saturating_duration_since(t) <= COSTLY_WITHIN) {
            self.grow = region::covering(rects, self.mailbox.width, self.mailbox.height);
        }
        self.costly_at = Some(started);
    }

    /// Whether the client has presented far fewer frames than were sent for [`CLIENT_SHORT_SECONDS`] reports in a row
    /// (its AV1 decoding cannot keep up). A client without H.264 has nothing to fall back to.
    fn client_falls_behind(&mut self) -> bool {
        let presented = self.mailbox.client_presented.swap(NO_REPORT, Ordering::Relaxed);
        // The first seconds of an episode mix in frames from before it, on both sides of the comparison.
        let settled = self.video_since.is_some_and(|t| t.elapsed() >= CLIENT_SETTLE);
        if presented == NO_REPORT || !self.can_fall_back() || !settled {
            return false;
        }
        self.client_short = if falls_short(presented, self.sent_last_second) { self.client_short + 1 } else { 0 };
        self.client_short >= CLIENT_SHORT_SECONDS
    }

    /// Records an AV1 encode that took `took` and says whether AV1 is behind the frame rate. A client without H.264
    /// has nothing to fall back to, so nothing is recorded for it.
    fn av1_falls_behind(&mut self, took: Duration) -> bool {
        if !self.can_fall_back() {
            return false;
        }
        sample_over_budget(took, AV1_SAMPLES, self.frame_interval(), &mut self.av1_times)
    }

    fn record(&mut self, took: Duration, bytes: usize) {
        let stats = &mut self.stats;
        let since = *stats.since.get_or_insert_with(Instant::now);
        stats.frames += 1;
        stats.bytes = stats.bytes.saturating_add(bytes as u32);
        stats.encode_us.push_back(took.as_micros() as u32);
        if stats.encode_us.len() > ENCODE_SAMPLES {
            stats.encode_us.pop_front();
        }
        if since.elapsed() >= Duration::from_secs(1) {
            let mut sorted: Vec<u32> = stats.encode_us.iter().copied().collect();
            sorted.sort_unstable();
            let pick = |q: f64| sorted.get(((sorted.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0);
            let message = Message::ServerStats {
                server_time_us: super::clock_us(),
                frames: stats.frames,
                bytes: stats.bytes,
                encode_us_p50: pick(0.5),
                encode_us_p95: pick(0.95),
            };
            self.sent_last_second = stats.video_frames;
            *stats = Stats::default();
            send(&self.out, &self.mailbox, &message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::cursor::Pointer;
    use crate::stream::viewer::Out;
    use crate::stream::yuv::Yuv;
    use display_protocol::Direction;
    use display_protocol::frame::Link;

    #[test]
    fn av1_falls_behind_when_its_median_encode_exceeds_the_budget() {
        let interval = Duration::from_micros(16_667);
        assert!(!over_budget(vec![6_000; AV1_SAMPLES], interval));
        assert!(over_budget(vec![11_000; AV1_SAMPLES], interval));
        // A few slow frames do not move the median.
        let mut mixed = vec![6_000; AV1_SAMPLES];
        mixed[..5].fill(40_000);
        assert!(!over_budget(mixed, interval));
    }

    #[test]
    fn codecs_follow_the_choice_the_client_and_the_build() {
        let both = CODEC_AV1 | CODEC_H264;
        assert_eq!(codec_order(CodecChoice::Auto, both)[0], Codec::Av1);
        assert_eq!(codec_order(CodecChoice::TilesOnly, both), vec![]);
        if H264_BUILT {
            assert_eq!(codec_order(CodecChoice::H264, both), vec![Codec::H264, Codec::Av1]);
            assert_eq!(codec_order(CodecChoice::Auto, CODEC_H264), vec![Codec::H264]);
        } else {
            // Without the encoder, H.264 is never chosen; a client that decodes only H.264 gets lossless tiles.
            assert_eq!(codec_order(CodecChoice::H264, both), vec![Codec::Av1]);
            assert_eq!(codec_order(CodecChoice::Auto, CODEC_H264), vec![]);
        }
    }

    #[test]
    fn a_client_falls_short_below_four_fifths_of_enough_frames() {
        assert!(falls_short(42, 58));
        assert!(!falls_short(50, 58));
        assert!(!falls_short(0, 19), "too few frames to judge");
        assert!(!falls_short(u32::MAX - 1, u32::MAX));
    }

    #[test]
    fn every_region_reaches_the_video_encoder_as_its_own_picture() {
        let (width, height) = (200usize, 130usize);
        let frame: Vec<u32> =
            (0..width * height).map(|i| 0xff00_0000 | (i as u32).wrapping_mul(2654435761) >> 8).collect();
        let planes = |yuv: &Yuv| {
            let p = &yuv.planes;
            (p.y_plane.borrow().to_vec(), p.u_plane.borrow().to_vec(), p.v_plane.borrow().to_vec())
        };
        // Inside, the whole width at the bottom, and on the bottom edge off the left one.
        for region in [Rect::new(64, 2, 70, 64), Rect::new(0, 66, 200, 64), Rect::new(64, 66, 136, 64)] {
            let (x, y, w, h) = (region.x as usize, region.y as usize, region.w as usize, region.h as usize);
            let packed: Vec<u32> = (y..y + h).flat_map(|row| frame[row * width + x..][..w].iter().copied()).collect();
            let (mut got, mut want) = (Yuv::new(w as u32, h as u32), Yuv::new(w as u32, h as u32));
            let mut crop = Vec::new();
            let (pixels, stride) = region_pixels(&frame, width, &region, &mut crop);
            got.convert(pixels, stride).unwrap();
            want.convert(&packed, w).unwrap();
            assert!(planes(&got) == planes(&want), "{region:?}");
        }
    }

    /// An encoder of a 512x256 output where a clip has played in the top-left tile for a while, its frames, and a
    /// snapshot where noise fills the right half at once: too costly for lossless motion, so video starts, over the
    /// clip alone.
    fn noise_beside_a_clip() -> (Encoder, std::sync::mpsc::Receiver<Out>, Snapshot) {
        let (width, height) = (512, 256);
        let (mut encoder, frames) = encoder_of(width, height);
        encoder.codec = Codec::Av1;
        let now = Instant::now();
        for ms in (0..=600u16).step_by(40).rev() {
            encoder.activity.mark(&[CLIP], now - Duration::from_millis(ms.into()));
        }
        let mut next = noise();
        let mut pixels = vec![0u32; (width * height) as usize];
        for y in 0..height {
            for x in 256..width {
                pixels[(y * width + x) as usize] = next();
            }
        }
        let snapshot = Snapshot {
            time_us: 0,
            pixels: Arc::new(pixels),
            damage: Vec::new(),
            motion_until: Some(Instant::now() + Duration::from_secs(1)),
        };
        (encoder, frames, snapshot)
    }

    /// An encoder of a `width` x `height` output, and what it sends.
    fn encoder_of(width: i32, height: i32) -> (Encoder, std::sync::mpsc::Receiver<Out>) {
        let (out, frames) = std::sync::mpsc::channel();
        (Encoder::new(Arc::new(Mailbox::new(width, height)), out), frames)
    }

    /// An encoder of `mailbox` running on its own thread, and what it sends.
    fn spawn_encoder(mailbox: &Arc<Mailbox>) -> (std::sync::mpsc::Receiver<Out>, std::thread::JoinHandle<()>) {
        let (out, frames) = std::sync::mpsc::channel();
        let mailbox = mailbox.clone();
        (frames, std::thread::spawn(move || Encoder::new(mailbox, out).run()))
    }

    const CLIP: Rect = Rect { x: 0, y: 0, w: 64, h: 64 };
    const NOISE: Rect = Rect { x: 256, y: 0, w: 256, h: 256 };

    /// How many times the rectangles `pick` takes from the messages in `frames` covered each pixel of a `width`-wide
    /// output of `pixels` pixels.
    fn coverage(
        frames: &std::sync::mpsc::Receiver<Out>,
        width: usize,
        pixels: usize,
        pick: impl Fn(Message) -> Vec<display_protocol::Rect>,
    ) -> Vec<u8> {
        let mut covered = vec![0u8; pixels];
        for out in frames.try_iter() {
            let Out::Now(frame) = out else { continue };
            let Link::Relay(message) = Link::decode(&frame[4..], Direction::ServerToClient).expect("within the bound")
            else {
                continue;
            };
            for r in pick(message) {
                for y in r.y..r.y + r.h {
                    covered[y as usize * width + r.x as usize..][..r.w as usize].iter_mut().for_each(|c| *c += 1);
                }
            }
        }
        covered
    }

    /// How many times tiles in `frames` covered each pixel of a 512x256 output.
    fn tile_coverage(frames: &std::sync::mpsc::Receiver<Out>) -> Vec<u8> {
        coverage(frames, 512, 512 * 256, |m| if let Message::Tiles { rects, .. } = m { rects } else { Vec::new() })
    }

    /// Pseudo-random opaque pixels.
    fn noise() -> impl FnMut() -> u32 {
        let mut seed = 0x2545_f491_u32;
        move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed | 0xff00_0000
        }
    }

    #[test]
    fn a_change_away_from_what_moves_goes_as_tiles_when_video_starts() {
        let (mut encoder, frames, snapshot) = noise_beside_a_clip();
        assert!(encoder.encode(&snapshot, false, vec![CLIP, NOISE]));
        assert_eq!(encoder.region, Some(CLIP));
        let covered = tile_coverage(&frames);
        let missed = (0..256).flat_map(|y| (256..512).map(move |x| (x, y))).filter(|&(x, y)| covered[y * 512 + x] == 0);
        assert_eq!(missed.count(), 0, "every pixel of the noise went as tiles");
    }

    #[test]
    fn a_refresh_around_the_video_sends_each_tile_once() {
        let (mut encoder, frames, snapshot) = noise_beside_a_clip();
        assert!(encoder.encode(&snapshot, true, vec![CLIP, NOISE]));
        assert_eq!(encoder.region, Some(CLIP));
        let covered = tile_coverage(&frames);
        let outside = |i: &usize| !(i % 512 < 64 && i / 512 < 64);
        assert!((0..512 * 256).filter(outside).all(|i| covered[i] == 1), "every pixel around the video went once");
    }

    #[test]
    fn every_epoch_opens_with_one_full_refresh_unasked() {
        let mailbox = Arc::new(Mailbox::new(512, 256));
        let pixels = Arc::new(vec![0xff20_4060; 512 * 256]);
        mailbox.offer(Arc::new(Snapshot { time_us: 0, pixels, damage: Vec::new(), motion_until: None }));
        mailbox.pending.lock().unwrap().hello = Some(CODEC_AV1);
        let (frames, encoder) = spawn_encoder(&mailbox);
        // A connect, then, once it has gone quiet, a settings change: each configuration, then its refresh, and nothing
        // more without damage.
        let mut epochs: Vec<(u32, Vec<u64>, usize)> = Vec::new();
        loop {
            let out = frames.recv_timeout(Duration::from_millis(300));
            let Ok(out) = out else {
                if !matches!(out, Err(std::sync::mpsc::RecvTimeoutError::Timeout)) || epochs.len() != 1 {
                    break;
                }
                let mut pending = mailbox.pending.lock().unwrap();
                (pending.settings.max_fps, pending.settings_changed) = (30, true);
                drop(pending);
                mailbox.wake.notify_one();
                continue;
            };
            let Out::Now(frame) = out else { continue };
            match Link::decode(&frame[4..], Direction::ServerToClient) {
                Ok(Link::Relay(Message::Config { epoch, .. })) => epochs.push((epoch, Vec::new(), 0)),
                Ok(Link::Relay(Message::Tiles { epoch, revision, rects, .. })) => {
                    acknowledge(&mailbox, revision);
                    let (current, revisions, area) = epochs.last_mut().expect("tiles after a configuration");
                    assert_eq!(epoch, *current);
                    revisions.push(revision);
                    *area += rects.iter().map(|r| (r.w * r.h) as usize).sum::<usize>();
                }
                _ => {}
            }
        }
        mailbox.close();
        encoder.join().unwrap();
        assert_eq!(epochs.len(), 2, "{epochs:?}");
        for (epoch, revisions, area) in epochs {
            assert_eq!((revisions.len(), area), (1, 512 * 256), "epoch {epoch} sent the screen once");
        }
    }

    /// What a client does on each media message: acknowledges its revision.
    fn acknowledge(mailbox: &Mailbox, revision: u64) {
        mailbox.pending.lock().unwrap().flight.acked(revision, Instant::now()).expect("a revision sent");
        mailbox.wake.notify_one();
    }

    /// A custom pointer of 60 KiB the person keeps moving (a new position every 5 ms) takes turns with the picture
    /// (a change every 16 ms), never every turn: both keep coming, each pointer acknowledged like media.
    #[test]
    fn a_large_moving_pointer_does_not_starve_the_picture() {
        let mailbox = Arc::new(Mailbox::new(256, 128));
        mailbox.pending.lock().unwrap().hello = Some(0);
        let (frames, encoder) = spawn_encoder(&mailbox);
        let image: Arc<[u8]> = vec![7u8; 60 * 1024].into();
        let (mut tiles, mut pointers) = (0, 0);
        let start = Instant::now();
        let mut i = 0u32;
        while start.elapsed() < Duration::from_secs(1) {
            i += 1;
            let pointer = Pointer { x: (i % 200) as u16, y: 10, shape: 255, hot_x: 0, hot_y: 0, image: image.clone() };
            mailbox.offer_pointer(pointer);
            if i.is_multiple_of(3) {
                let pixels = Arc::new(vec![0xff00_0000 | i; 256 * 128]);
                let damage = vec![Rect::new(0, 0, 256, 128)];
                mailbox.offer(Arc::new(Snapshot { time_us: 0, pixels, damage, motion_until: None }));
            }
            // The writer and the client: everything queued written, and acknowledged.
            for out in frames.try_iter() {
                let Out::Now(frame) = out else { continue };
                mailbox.queued.fetch_sub(frame.len(), Ordering::Relaxed);
                match Link::decode(&frame[4..], Direction::ServerToClient) {
                    Ok(Link::Relay(Message::Tiles { revision, .. })) => {
                        (tiles, _) = (tiles + 1, acknowledge(&mailbox, revision))
                    }
                    Ok(Link::Relay(Message::Cursor { revision, .. })) => {
                        (pointers, _) = (pointers + 1, acknowledge(&mailbox, revision))
                    }
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        mailbox.close();
        encoder.join().unwrap();
        // Tiles-only clients get 15 updates a second at most.
        assert!(tiles >= 8, "{tiles} updates of the picture in a second, beside {pointers} pointers");
        assert!(pointers >= 8, "{pointers} pointers in a second");
    }

    /// A client that stops acknowledging media gets no more once what it was sent outlives the allowance, however
    /// much changes; its next acknowledgement brings the current picture at once.
    #[test]
    fn capture_waits_for_acknowledgements() {
        let mailbox = Arc::new(Mailbox::new(256, 128));
        mailbox.pending.lock().unwrap().hello = Some(0);
        let (frames, encoder) = spawn_encoder(&mailbox);
        let all = Rect::new(0, 0, 256, 128);
        let offer = |shade: u32| {
            let pixels = Arc::new(vec![0xff00_0000 | shade; 256 * 128]);
            mailbox.offer(Arc::new(Snapshot { time_us: 0, pixels, damage: vec![all], motion_until: None }));
        };
        // A new picture every 20 ms for a second, nothing acknowledged.
        let media = |frames: &std::sync::mpsc::Receiver<Out>| {
            let mut revisions = Vec::new();
            for out in frames.try_iter() {
                let Out::Now(frame) = out else { continue };
                if let Ok(Link::Relay(Message::Tiles { revision, .. })) =
                    Link::decode(&frame[4..], Direction::ServerToClient)
                {
                    revisions.push(revision);
                }
            }
            revisions
        };
        let start = Instant::now();
        let mut shade = 0;
        while start.elapsed() < Duration::from_secs(1) {
            shade += 1;
            offer(shade);
            std::thread::sleep(Duration::from_millis(20));
        }
        let sent = media(&frames);
        assert!(!sent.is_empty() && sent.len() <= 12, "{} updates went unacknowledged", sent.len());
        // Acknowledged: the current picture goes at once.
        acknowledge(&mailbox, *sent.last().unwrap());
        let resumed = Instant::now();
        let next = loop {
            if let Some(revision) = media(&frames).last().copied() {
                break revision;
            }
            assert!(resumed.elapsed() < Duration::from_secs(1), "nothing after the acknowledgement");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(next > *sent.last().unwrap());
        mailbox.close();
        encoder.join().unwrap();
    }

    #[test]
    fn a_connect_with_settings_before_the_hello_opens_one_epoch_with_one_full_refresh() {
        // The real order: the admin attaches and sends its settings at once, and the client's hello comes after.
        let mailbox = Arc::new(Mailbox::new(512, 256));
        let mut pending = mailbox.pending.lock().unwrap();
        (pending.settings.max_fps, pending.settings_changed) = (30, true);
        drop(pending);
        let pixels = Arc::new(vec![0xff20_4060; 512 * 256]);
        mailbox.offer(Arc::new(Snapshot { time_us: 0, pixels, damage: Vec::new(), motion_until: None }));
        mailbox.pending.lock().unwrap().hello = Some(CODEC_AV1);
        let (frames, encoder) = spawn_encoder(&mailbox);
        let (mut epochs, mut area) = (Vec::new(), 0);
        while let Ok(out) = frames.recv_timeout(Duration::from_millis(300)) {
            let Out::Now(frame) = out else { continue };
            match Link::decode(&frame[4..], Direction::ServerToClient) {
                Ok(Link::Relay(Message::Config { epoch, max_fps, .. })) => epochs.push((epoch, max_fps)),
                Ok(Link::Relay(Message::Tiles { rects, .. })) => {
                    area += rects.iter().map(|r| (r.w * r.h) as usize).sum::<usize>();
                }
                _ => {}
            }
        }
        mailbox.close();
        encoder.join().unwrap();
        assert_eq!(epochs, vec![(1, 30)], "one epoch, with the settings");
        assert_eq!(area, 512 * 256, "the screen went once");
    }

    #[test]
    fn an_epoch_configured_mid_motion_refreshes_when_motion_ends() {
        let (mut encoder, frames, snapshot) = noise_beside_a_clip();
        encoder.codecs = CODEC_AV1;
        assert!(encoder.encode(&snapshot, true, vec![CLIP, NOISE]));
        assert!(encoder.in_video);
        // A new epoch while video shows (as the switch from AV1 to H.264 makes), and the next frame is still.
        assert!(encoder.configure(CODEC_AV1, encoder.settings));
        frames.try_iter().for_each(drop);
        let still = Snapshot { motion_until: None, ..snapshot };
        assert!(encoder.encode(&still, false, Vec::new()));
        assert!(tile_coverage(&frames).iter().all(|c| *c == 1), "the new epoch got the whole screen once");
    }

    #[test]
    fn an_epoch_configured_while_motion_goes_on_opens_with_a_keyframe_and_tiles_around_it() {
        let (mut encoder, frames, snapshot) = noise_beside_a_clip();
        encoder.codecs = CODEC_AV1;
        assert!(encoder.encode(&snapshot, true, vec![CLIP, NOISE]));
        assert!(encoder.configure(CODEC_AV1, encoder.settings));
        frames.try_iter().for_each(drop);
        // The clip goes on moving, and nothing else changes.
        encoder.activity.mark(&[CLIP], Instant::now());
        assert!(encoder.encode(&snapshot, false, vec![CLIP]));
        let covered = coverage(&frames, 512, 512 * 256, |m| match m {
            Message::Tiles { epoch, rects, .. } if epoch == encoder.epoch => rects,
            Message::Video { epoch, keyframe: true, rect, .. } if epoch == encoder.epoch => vec![rect],
            _ => Vec::new(),
        });
        assert!(covered.iter().all(|c| *c == 1), "a keyframe and the tiles around it covered the screen once");
    }

    /// Compositions every `every_ms` over `[from_ms, to_ms)` after `start`, each damaging `damage`.
    fn composed(
        list: &mut VecDeque<(Instant, Vec<Rect>)>,
        start: Instant,
        span: (u64, u64),
        every_ms: u64,
        damage: Rect,
    ) {
        for ms in (span.0..span.1).step_by(every_ms as usize) {
            list.push_back((start + Duration::from_millis(ms), vec![damage]));
        }
        list.make_contiguous().sort_by_key(|(t, _)| *t);
    }

    #[test]
    fn the_source_rate_of_a_region_needs_a_whole_window_of_its_frames() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let (clip, elsewhere) = (Rect::new(0, 0, 64, 64), Rect::new(512, 512, 64, 64));
        let mut list = VecDeque::new();
        composed(&mut list, start, (0, 500), 33, clip);
        assert_eq!(source_rate(&list, &clip, at(500)), None, "not yet a whole window");
        composed(&mut list, start, (500, 2000), 33, clip);
        assert_eq!(source_rate(&list, &clip, at(2000)), Some(30));
        assert_eq!(source_rate(&list, &elsewhere, at(2000)), None, "nothing there");
        // A clip that appears beside motion that goes on is not measured until it has had frames for a window.
        let mut busy = VecDeque::new();
        composed(&mut busy, start, (0, 2000), 16, elsewhere);
        composed(&mut busy, start, (1700, 2000), 16, clip);
        assert_eq!(source_rate(&busy, &clip, at(2000)), None);
        // Nor after an old repaint of the whole screen: the clip's frames began after a pause.
        let mut after_paint = VecDeque::new();
        composed(&mut after_paint, start, (0, 1), 1, Rect::new(0, 0, 1280, 800));
        composed(&mut after_paint, start, (1700, 2000), 16, clip);
        assert_eq!(source_rate(&after_paint, &clip, at(2000)), None);
        // Nor once its frames have stopped.
        assert_eq!(source_rate(&list, &clip, at(2500)), None);
        // A history cut short (the oldest kept frame is within the window) measures nothing.
        let short: VecDeque<_> = list.iter().filter(|(t, _)| *t > at(1500)).cloned().collect();
        assert_eq!(source_rate(&short, &clip, at(2000)), None);
    }

    #[test]
    fn the_encoder_restarts_for_the_rate_the_source_gives_its_region_now_and_then() {
        let (mut encoder, _frames) = encoder_of(256, 256);
        let mailbox = encoder.mailbox.clone();
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let region = Rect::new(0, 0, 64, 64);
        (encoder.video_fps, encoder.rate_fit_at) = (60, Some(start));
        // Frames `every_ms` from `from_ms`, checked at each: when a restart is first due.
        let first_refit = |encoder: &mut Encoder, from_ms: u64, to_ms: u64, every_ms: u64| {
            (from_ms..to_ms).step_by(every_ms as usize).find(|&ms| {
                let mut pending = mailbox.pending.lock().unwrap();
                composed(&mut pending.compositions, start, (ms, ms + 1), 1, region);
                drop(pending);
                encoder.note_video_frame(at(ms));
                encoder.rate_refit(at(ms), &region)
            })
        };
        // A 30 fps video against an encoder started for 60: known after a window, a restart 2 s later, held to 5 s
        // after the last start.
        let first = first_refit(&mut encoder, 0, 6000, 33).expect("a restart");
        assert!((5000..5100).contains(&first), "at {first} ms");
        assert_eq!(encoder.region_fps(at(first), &region), Some(VIDEO_FPS));
        // Restarted for 30; motion at 60 then restarts it for the cap, again no sooner than 5 s after.
        (encoder.video_fps, encoder.rate_fit_at, encoder.rate_off_since) = (VIDEO_FPS, Some(at(6000)), None);
        let again = first_refit(&mut encoder, 6000, 12_000, 16).expect("a restart");
        assert!((11_000..11_100).contains(&again), "at {again} ms");
        // At the rate it was started for, never.
        (encoder.video_fps, encoder.rate_fit_at, encoder.rate_off_since) = (60, None, None);
        assert_eq!(first_refit(&mut encoder, 12_000, 20_000, 16), None);
        // A video that encodes half of a 30 fps source's frames stays at the cap.
        let (mut behind, _) = encoder_of(256, 256);
        composed(&mut behind.mailbox.pending.lock().unwrap().compositions, start, (20_000, 23_000), 33, region);
        for ms in (20_000..23_000).step_by(66) {
            behind.note_video_frame(at(ms));
        }
        assert_eq!(behind.region_fps(at(23_000), &region), Some(60));
        // A cap below the video rate is the only rate.
        encoder.settings.max_fps = 20;
        encoder.video_fps = 20;
        assert_eq!(first_refit(&mut encoder, 20_000, 30_000, 33), None);
    }

    #[test]
    fn a_source_between_the_two_rates_keeps_the_encoder_where_it_is() {
        let (mut encoder, _) = encoder_of(256, 256);
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let region = Rect::new(0, 0, 64, 64);
        // 40 frames a second, all of them sent.
        composed(&mut encoder.mailbox.pending.lock().unwrap().compositions, start, (0, 2000), 25, region);
        (0..2000).step_by(25).for_each(|ms| encoder.note_video_frame(at(ms)));
        encoder.video_fps = VIDEO_FPS;
        assert_eq!(encoder.region_fps(at(2000), &region), Some(VIDEO_FPS));
        encoder.video_fps = 60;
        assert_eq!(encoder.region_fps(at(2000), &region), Some(60));
    }

    #[test]
    fn a_region_that_grows_back_soon_after_shrinking_shrinks_only_the_slow_way() {
        let (mut encoder, _) = encoder_of(1280, 800);
        encoder.codec = Codec::Av1;
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let (whole, video) = (Rect::new(0, 0, 1280, 800), Rect::new(0, 0, 1280, 768));
        encoder.video = Some(start_video(Codec::Av1, 64, 64, encoder.settings, 60).unwrap());
        (encoder.region, encoder.in_video) = (Some(whole), true);
        // The video moves for good; an overlay at the bottom edge moved, then went: the region leaves it soon.
        let play = |encoder: &mut Encoder, from: u64, to: u64, strip: bool| {
            for ms in (from..to).step_by(40) {
                encoder.activity.mark(&[video], at(ms));
                if strip {
                    encoder.activity.mark(&[Rect::new(0, 768, 1280, 32)], at(ms));
                }
            }
        };
        play(&mut encoder, 0, 1000, true);
        play(&mut encoder, 1000, 4000, false);
        let shrink = (1000..4000).step_by(40).find_map(|ms| encoder.next_region(at(ms)).map(|r| (ms, r)));
        let (when, to) = shrink.expect("a shrink");
        assert_eq!(to, video);
        assert!(when < 3000, "at {when} ms");
        (encoder.region, encoder.region_moved_at) = (Some(video), Some(at(when)));
        // The overlay comes back: the region grows again, and from then on keeps it for SHRINK_AFTER.
        play(&mut encoder, 4000, 5000, true);
        assert_eq!(encoder.next_region(at(5000)), Some(whole));
        (encoder.region, encoder.region_moved_at) = (Some(whole), Some(at(5000)));
        play(&mut encoder, 5000, 9000, false);
        let again = (5000..9000).step_by(40).find_map(|ms| encoder.next_region(at(ms)));
        assert_eq!(again, None, "no quick shrink after growing back");
    }

    #[test]
    fn frames_sent_for_one_region_do_not_count_for_another() {
        let (mut encoder, _) = encoder_of(256, 256);
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let (a, b) = (Rect::new(0, 0, 64, 64), Rect::new(128, 128, 64, 64));
        let grown = Rect::new(64, 64, 192, 192);
        // A 30 fps source on B for two seconds; 30 frames a second were sent, for A.
        composed(&mut encoder.mailbox.pending.lock().unwrap().compositions, start, (0, 2000), 33, b);
        let fill = |encoder: &mut Encoder| (0..2000).step_by(33).for_each(|ms| encoder.note_video_frame(at(ms)));
        fill(&mut encoder);
        encoder.video_fps = 60;
        assert_eq!(encoder.start_fps(at(2000), &b, Some(a), true), 60, "a move to B keeps the encoder's rate");
        fill(&mut encoder);
        assert_eq!(encoder.start_fps(at(2000), &b, Some(b), true), VIDEO_FPS, "a restart in place keeps its count");
        // A region that grows over B is not judged by what was sent for B: it keeps the encoder's rate until it has a
        // window of its own; one within the region that was sent for is.
        encoder.video_fps = VIDEO_FPS;
        assert_eq!(encoder.start_fps(at(2000), &grown, Some(b), true), VIDEO_FPS);
        encoder.video_fps = 60;
        assert_eq!(encoder.start_fps(at(2000), &grown, Some(b), true), 60);
        fill(&mut encoder);
        assert_eq!(encoder.start_fps(at(2000), &b, Some(grown), true), VIDEO_FPS);
    }

    #[test]
    fn only_frames_with_new_damage_of_the_region_count_as_keeping_up() {
        let (mut encoder, _frames) = encoder_of(256, 256);
        encoder.codec = Codec::Av1;
        let region = Rect::new(0, 0, 64, 64);
        encoder.video = Some(start_video(Codec::Av1, 64, 64, encoder.settings, 60).unwrap());
        (encoder.region, encoder.in_video, encoder.keyframe_at) = (Some(region), true, Some(Instant::now()));
        let snapshot =
            Snapshot { time_us: 0, pixels: Arc::new(vec![0; 256 * 256]), damage: Vec::new(), motion_until: None };
        let now = Instant::now();
        encoder.activity.mark(&[region], now);
        // A refresh (a keyframe) with no new damage there, and damage elsewhere: neither counts.
        assert!(encoder.encode_video(&snapshot, true, now, Vec::new()));
        assert!(encoder.encode_video(&snapshot, false, now, vec![Rect::new(128, 128, 64, 64)]));
        assert!(encoder.video_sent.is_empty());
        assert!(encoder.encode_video(&snapshot, false, now, vec![region]));
        assert_eq!(encoder.video_sent.len(), 1);
    }

    #[test]
    fn a_still_change_inside_the_region_gets_one_keyframe() {
        let (mut encoder, frames) = encoder_of(256, 256);
        encoder.codec = Codec::Av1;
        let output = Rect::new(0, 0, 256, 256);
        encoder.video = Some(start_video(Codec::Av1, 256, 256, encoder.settings, 60).unwrap());
        (encoder.region, encoder.in_video) = (Some(output), true);
        let now = Instant::now();
        encoder.keyframe_at = Some(now - Duration::from_secs(2));
        // Clips in opposite corners hold the region; four tiles between them changed 600 ms ago and stayed still.
        let corners = [Rect::new(0, 0, 64, 64), Rect::new(192, 192, 64, 64)];
        encoder.activity.mark(&[Rect::new(64, 64, 128, 128)], now - Duration::from_millis(600));
        encoder.activity.mark(&corners, now);
        let snapshot =
            Snapshot { time_us: 0, pixels: Arc::new(vec![0; 256 * 256]), damage: Vec::new(), motion_until: None };
        assert!(encoder.encode_video(&snapshot, false, now, corners.to_vec()));
        assert_eq!(encoder.keyframe_at, Some(now), "the still change got a keyframe");
        assert_eq!(encoder.region, Some(output), "of the same region");
        // Not again before REPAINT_AFTER, nor for a change it already repainted.
        let next = now + Duration::from_millis(100);
        encoder.activity.mark(&corners, next);
        assert!(encoder.encode_video(&snapshot, false, next, corners.to_vec()));
        assert_eq!(encoder.keyframe_at, Some(now));
        let later = now + REPAINT_AFTER;
        encoder.activity.mark(&corners, later);
        assert!(encoder.encode_video(&snapshot, false, later, corners.to_vec()));
        assert_eq!(encoder.keyframe_at, Some(now));
        assert!(frames.try_iter().count() >= 3);
    }

    /// A clip in the middle of a 512x320 output goes fullscreen while the encoder, held back by a slow link, encodes
    /// only every 330 ms, the source composing a new picture every 42 ms (and, with `watch`, the encoder observing each,
    /// as it does while held back): how long until the video's region covers the output, if within 5 s.
    fn fullscreen_while_throttled(watch: bool) -> Option<Duration> {
        let (width, height) = (512, 320);
        let (mut encoder, _frames) = encoder_of(width, height);
        encoder.codec = Codec::Av1;
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let (clip, output) = (Rect::new(128, 64, 256, 128), Rect::new(0, 0, width, height));
        // The clip has played as video of its own region for a second.
        for ms in (0..1000).step_by(42) {
            encoder.activity.mark(&[clip], at(ms));
        }
        encoder.video = Some(start_video(Codec::Av1, 256, 128, encoder.settings, 30).unwrap());
        (encoder.region, encoder.in_video, encoder.keyframe_at) = (Some(clip), true, Some(at(0)));
        let mut next = noise();
        let pictures: Vec<Arc<Vec<u32>>> =
            (0..3).map(|_| Arc::new((0..width * height).map(|_| next()).collect())).collect();
        // What the encoder takes as damage: the output's tiles.
        let tiles = region::tiles_outside(&output, None);
        let mut encoded_at = 0;
        for (i, ms) in (1000..6000u64).step_by(42).enumerate() {
            let snapshot = Snapshot {
                time_us: ms * 1000,
                pixels: pictures[i % 3].clone(),
                damage: vec![output],
                motion_until: None,
            };
            if ms - encoded_at >= 330 {
                encoded_at = ms;
                encoder.activity.observe(&snapshot.pixels, &tiles, at(ms));
                assert!(encoder.encode_video(&snapshot, false, at(ms), tiles.clone()));
                if encoder.region == Some(output) {
                    return Some(Duration::from_millis(ms - 1000));
                }
            } else if watch {
                encoder.observe(&snapshot, &[output], at(ms));
            }
        }
        None
    }

    /// The fullscreen trap: a throttled encoder that saw the new motion only when it encoded never let the region grow,
    /// and sent everything around it as lossless tiles, which kept it throttled. What changes outside the region at a
    /// greater cost than video, twice in a row, grows it; and changes observed at the compositor's pace while held back
    /// are motion like any other.
    #[test]
    fn a_video_that_goes_fullscreen_under_a_slow_link_soon_takes_the_whole_screen() {
        for watch in [false, true] {
            let took = fullscreen_while_throttled(watch);
            assert!(took.is_some_and(|t| t <= Duration::from_secs(1)), "watching {watch}: {took:?}");
        }
    }

    /// A 512x320 output where a clip in the middle plays as video of its own region, the source composing a picture
    /// every 42 ms from `picture(i, ms)` for 4 s and the whole output damaged each time, observed at every composition
    /// (as while held back) and encoded every 330 ms: the region at the end.
    fn clip_beside(picture: impl Fn(usize, u64) -> Vec<u32>) -> Rect {
        let (width, height) = (512, 320);
        let (mut encoder, _frames) = encoder_of(width, height);
        encoder.codec = Codec::Av1;
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let clip = Rect::new(128, 64, 256, 128);
        for ms in (0..1000).step_by(42) {
            encoder.activity.mark(&[clip], at(ms));
        }
        encoder.video = Some(start_video(Codec::Av1, 256, 128, encoder.settings, 30).unwrap());
        (encoder.region, encoder.in_video, encoder.keyframe_at) = (Some(clip), true, Some(at(0)));
        let tiles = region::tiles_outside(&Rect::new(0, 0, width, height), None);
        let mut encoded_at = 0;
        for (i, ms) in (1000..5000u64).step_by(42).enumerate() {
            let pixels = Arc::new(picture(i, ms));
            let snapshot = Snapshot { time_us: ms * 1000, pixels, damage: tiles.clone(), motion_until: None };
            if ms - encoded_at >= 330 {
                encoded_at = ms;
                encoder.activity.observe(&snapshot.pixels, &tiles, at(ms));
                assert!(encoder.encode_video(&snapshot, false, at(ms), tiles.clone()));
            } else {
                encoder.observe(&snapshot, &tiles, at(ms));
            }
        }
        encoder.region.expect("a region")
    }

    /// A 512x320 picture: the clip's noise (another every frame) over `background`, and `corner` in the top-left tile.
    fn clip_picture(i: usize, background: u32, corner: u32) -> Vec<u32> {
        let mut seed = 0x9e37_79b9_u32.wrapping_mul(i as u32 + 1);
        let mut pixels = vec![background; 512 * 320];
        for y in 64..192 {
            for x in 128..384 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                pixels[y * 512 + x] = seed | 0xff00_0000;
            }
        }
        for y in 0..64 {
            pixels[y * 512..y * 512 + 64].fill(corner);
        }
        pixels
    }

    /// A new page around a playing clip (a costly change, once) stays exact: the region keeps to the clip.
    #[test]
    fn a_new_page_around_a_playing_clip_does_not_grow_its_region() {
        let mut next = noise();
        let page: Vec<u32> = (0..512 * 320).map(|_| next()).collect();
        let region = clip_beside(|i, ms| {
            let mut pixels = clip_picture(i, 0xff10_2030, 0xff10_2030);
            // The page changes once, after a second, to rich content that stays.
            if ms >= 2000 {
                for (p, q) in pixels.iter_mut().zip(&page).filter(|(p, _)| **p == 0xff10_2030) {
                    *p = *q;
                }
            }
            pixels
        });
        assert_eq!(region, Rect::new(128, 64, 256, 128));
    }

    /// A clock beside a playing clip, ticking each second under damage that covers it at every composition, is no
    /// motion: the pixels say it changed once a second, not continuously.
    #[test]
    fn a_ticking_clock_damaged_with_a_clip_does_not_join_its_region() {
        let region = clip_beside(|i, ms| clip_picture(i, 0xff10_2030, 0xff00_0000 | ((ms / 1000) as u32 * 40)));
        assert_eq!(region, Rect::new(128, 64, 256, 128));
    }

    /// A refresh larger than an encode's budget goes in pieces, each about the budget, the first in the encode and the
    /// rest in drains; a tile that changed while it waited goes as it is now; and the screen ends exact, every tile once.
    #[test]
    fn a_refresh_larger_than_the_budget_goes_in_pieces_and_ends_exact() {
        let (width, height) = (512, 256);
        let (mut encoder, frames) = encoder_of(width, height);
        encoder.budget = 32 * 1024;
        let mut next = noise();
        let mut pixels: Vec<u32> = (0..width * height).map(|_| next()).collect();
        let snapshot = |pixels: &Vec<u32>| Snapshot {
            time_us: 0,
            pixels: Arc::new(pixels.clone()),
            damage: Vec::new(),
            motion_until: None,
        };
        // Every tile sent so far, decoded into `seen`, counted in `covered`; the bytes it took.
        let mut seen = vec![0u32; (width * height) as usize];
        let mut covered = vec![0u8; (width * height) as usize];
        let mut take = || {
            let mut bytes = 0;
            for out in frames.try_iter() {
                let Out::Now(frame) = out else { continue };
                bytes += frame.len();
                let Ok(Link::Relay(Message::Tiles { rects, raw_len, data, .. })) =
                    Link::decode(&frame[4..], Direction::ServerToClient)
                else {
                    continue;
                };
                let mut raw = Vec::new();
                std::io::Read::read_to_end(&mut flate2::read::DeflateDecoder::new(&data[..]), &mut raw).unwrap();
                assert_eq!(raw.len(), raw_len as usize);
                for (r, rgb) in rects.iter().zip(display_protocol::records::decode(&rects, &raw).unwrap()) {
                    for (i, px) in rgb.chunks(3).enumerate() {
                        let at = (r.y as usize + i / r.w as usize) * 512 + r.x as usize + i % r.w as usize;
                        seen[at] = 0xff00_0000 | u32::from(px[0]) << 16 | u32::from(px[1]) << 8 | u32::from(px[2]);
                        covered[at] += 1;
                    }
                }
            }
            bytes
        };
        assert!(encoder.encode(&snapshot(&pixels), true, Vec::new()));
        let mut sizes = vec![take()];
        // The bottom-right tile changes while it is owed.
        for y in 192..256 {
            pixels[y * 512 + 448..y * 512 + 512].fill(0xff12_3456);
        }
        let mut drains = 0;
        while !encoder.backlog.is_empty() {
            assert!(encoder.drain(&snapshot(&pixels)));
            drains += 1;
            assert!(drains < 100);
            sizes.push(take());
        }
        assert!(drains >= 4, "{drains} drains");
        assert!(sizes.iter().all(|b| *b < 32 * 1024 + 70 * 1024), "{sizes:?}");
        assert!(sizes[0] > 0, "the encode sent its piece at once");
        // The encode's piece and the drains together send every tile once, and the picture ends as it is now.
        assert!(covered.iter().all(|c| *c == 1), "a pixel sent {:?} times", covered.iter().find(|c| **c != 1));
        assert!(seen == pixels, "the final picture differs");
        assert_eq!(seen[255 * 512 + 511], 0xff12_3456, "the tile that changed while owed went as it is now");
    }

    /// When video ends, the lossless refresh covers the video's region, not the rest of the screen, which tiles kept
    /// exact meanwhile.
    #[test]
    fn leaving_video_refreshes_only_its_region() {
        let (mut encoder, frames, snapshot) = noise_beside_a_clip();
        assert!(encoder.encode(&snapshot, false, vec![CLIP, NOISE]));
        assert!(encoder.in_video && encoder.region == Some(CLIP));
        frames.try_iter().for_each(drop);
        let still = Snapshot { motion_until: None, ..snapshot };
        assert!(encoder.encode(&still, false, Vec::new()));
        let covered = tile_coverage(&frames);
        let inside = |i: usize| i % 512 < 64 && i / 512 < 64;
        assert!((0..512 * 256).all(|i| covered[i] == u8::from(inside(i))), "only the clip's region was refreshed");
        assert!(!encoder.in_video);
    }

    /// Video held by costly updates (or by the source's motion) ends once the picture has been still for a while: its
    /// region's exact refresh need not wait out the hold.
    #[test]
    fn video_held_by_costly_updates_ends_once_the_picture_is_still() {
        let (mut encoder, _frames) = encoder_of(256, 256);
        let now = Instant::now();
        let moving = Snapshot {
            time_us: 0,
            pixels: Arc::new(vec![0; 256 * 256]),
            damage: Vec::new(),
            motion_until: Some(now + Duration::from_secs(5)),
        };
        encoder.in_video = true;
        encoder.heavy_until = Some(now + Duration::from_secs(5));
        encoder.video_since = Some(now);
        assert!(!encoder.video_may_end(Some(&moving), now + Duration::from_secs(1)), "no change seen yet");
        encoder.activity.mark(&[Rect::new(0, 0, 64, 64)], now);
        assert!(!encoder.video_may_end(Some(&moving), now), "the picture changed just now");
        let later = now + region::ACTIVE_WINDOW;
        assert!(encoder.video_may_end(Some(&moving), later), "still for the active window, whatever holds it");
        encoder.in_video = false;
        assert!(!encoder.video_may_end(Some(&moving), later), "no video, nothing to end");
    }

    /// A frame of noise over a large output, at the highest bitrate and the lowest frame rate the settings allow: a
    /// budget per frame well past the message bound.
    #[test]
    fn every_message_of_a_costly_video_frame_fits_the_bound() {
        let (width, height) = (1920, 1080);
        let (mut encoder, frames) = encoder_of(width, height);
        encoder.codec = Codec::Av1;
        encoder.settings = Settings {
            codec: CodecChoice::Av1,
            bitrate_kbps: *display_protocol::frame::BITRATE_KBPS.end(),
            max_fps: *display_protocol::frame::MAX_FPS.start(),
        };
        let mut next = noise();
        let pixels = (0..width * height).map(|_| next()).collect();
        let snapshot = Snapshot { time_us: 0, pixels: Arc::new(pixels), damage: Vec::new(), motion_until: None };
        let output = Rect::new(0, 0, width, height);
        let now = Instant::now();
        encoder.activity.mark(&[output], now);
        assert!(encoder.encode_video(&snapshot, false, now, vec![output]));
        let covered = coverage(&frames, width as usize, (width * height) as usize, |m| match m {
            Message::Tiles { rects, .. } => rects,
            Message::Video { rect, .. } => vec![rect],
            _ => Vec::new(),
        });
        assert!(covered.iter().all(|c| *c > 0), "the frame reached the viewer whole");
        assert!(encoder.keyframe_owed, "the next video frame is a keyframe");
    }

    #[test]
    fn encodes_keep_the_frame_rate_cap_on_average() {
        let interval = Duration::from_micros(16_667);
        let slack = interval / 4;
        let start = Instant::now();
        // A source that is always ready: every encode starts as early as the slack allows.
        let (mut due, mut at) = (None, start);
        for _ in 0..600 {
            let next = next_due(due, at, interval);
            due = Some(next);
            at = next - slack;
        }
        assert!(at - start >= interval * 599 - slack, "600 encodes took {:?}", at - start);
        // After a pause the cadence restarts from now: the next encode starts at least half an interval later, never
        // in a burst of the missed ones.
        let late = at + Duration::from_secs(1);
        assert!(next_due(due, late, interval) - slack >= late + interval / 2);
    }
}
