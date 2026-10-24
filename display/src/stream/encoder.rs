//! A viewer's encoder: one thread that turns composed frames into this viewer's stream. It picks each epoch's codec,
//! sends lossless tiles for ordinary damage and for motion that tiles carry within the video's budget, video for the
//! rest of motion (AV1 only where the picture moves, see `region`; lossless tiles around it), paces itself by the
//! viewer's frame-rate cap, and reports its statistics.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use display_protocol::frame::CodecChoice;
use display_protocol::{CODEC_AV1, CODEC_H264, Codec, MAX_VIDEO_PAYLOAD, Message, Rect as WireRect, VERSION};

use super::Snapshot;
use super::av1::Av1;
use super::cursor::Pointer;
#[cfg(feature = "h264")]
use super::h264::H264;
use super::region::{self, Activity};
use super::tiles;
use super::viewer::{Mailbox, NO_REPORT, Outbox, QUEUE_BUDGET, Settings, send};
use crate::compose::Rect;

/// Lossless-only clients get at most this many updates per second.
const TILES_ONLY_FPS: u8 = 15;

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

fn cursor_message(epoch: u32, p: &Pointer) -> Message {
    Message::Cursor { epoch, x: p.x, y: p.y, shape: p.shape, hot_x: p.hot_x, hot_y: p.hot_y, image: p.image.to_vec() }
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

pub struct Encoder {
    mailbox: Arc<Mailbox>,
    out: Outbox,
    epoch: u32,
    codec: Codec,
    settings: Settings,
    video: Option<Video>,
    in_video: bool,
    sent: tiles::Sent,
    deflate: flate2::Compress,
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
    /// A region's pixels cut from the frame, when the video encoder cannot read them in place.
    crop: Vec<u32>,
    /// What a lossless trial that gave way to video recorded as sent without sending it.
    unsent: Vec<Rect>,
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
}

/// The window over which the source's frame rate is measured.
const RATE_WINDOW: Duration = Duration::from_secs(1);
/// The frame rate an encoder starts for when the source gives its region at most [`VIDEO_RATE_MAX`] frames a second, as
/// a video at 24, 25, or 30 frames a second does, and the video keeps up with it ([`KEPT_UP`]); else it starts for the
/// cap. Only these two, so a source slowed by the
/// display's own load moves the encoder one step at most.
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
/// RGB records (content too rich for a palette) a lossless motion frame may hold before its tiles are abandoned
/// without deflating them: about what the default deflate level compresses in 10 ms on one core when the content does
/// not compress at all (antialiased text takes about a quarter of that).
const LOSSLESS_RGB_BYTES: usize = 256 * 1024;
/// Updates below the motion detector's frame rate (a slideshow, a page that redraws every few hundred milliseconds)
/// count as motion once this many of them within [`HEAVY_WINDOW`] took more than the lossless motion budget (bytes or
/// encode time), and stay
/// motion until changes stop for [`HEAVY_HOLD`].
const HEAVY_UPDATES: usize = 3;
const HEAVY_WINDOW: Duration = Duration::from_secs(2);
const HEAVY_HOLD: Duration = Duration::from_secs(1);
/// While video shows, it moves to a smaller region once what moves has fitted in one this long, and moves at most once
/// per [`REGION_MOVE_EVERY`]: each move restarts the encoder (a few to tens of milliseconds) with a keyframe, which
/// under constant bitrate looks worse for a moment. What moves in a video or a game shifts from place to place, so the
/// region grows at once but keeps what it has for a while. Changes outside the region meanwhile go as lossless tiles.
const SHRINK_AFTER: Duration = Duration::from_secs(10);
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

/// A tiles message's size on the wire: its fixed fields, its rectangle table, and its records.
fn tile_bytes(message: &Message) -> usize {
    if let Message::Tiles { rects, data, .. } = message { 19 + 8 * rects.len() + data.len() } else { 0 }
}

/// Queues tiles messages for the viewer: their bytes, and whether every one was queued.
fn send_tiles(out: &Outbox, mailbox: &Mailbox, messages: &[Message]) -> (usize, bool) {
    messages
        .iter()
        .fold((0, true), |(bytes, ok), message| (bytes + tile_bytes(message), send(out, mailbox, message) & ok))
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
            crop: Vec::new(),
            unsent: Vec::new(),
            video_fps: 0,
            rate_fit_at: None,
            rate_off_since: None,
            video_sent: VecDeque::new(),
            video_sent_since: None,
            shrunk_at: None,
            flapping: false,
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
            if self.codec == Codec::None { self.settings.max_fps.min(TILES_ONLY_FPS) } else { self.settings.max_fps };
        Duration::from_secs_f64(1.0 / f64::from(fps.max(1)))
    }

    pub fn run(mut self) {
        loop {
            let work = {
                let mailbox = self.mailbox.clone();
                let mut pending = mailbox.pending.lock().unwrap();
                let mut drop_video = false;
                loop {
                    if pending.closed {
                        return;
                    }
                    // The pointer waits for room like frames do; the latest one wins meanwhile.
                    let room = self.mailbox.queued.load(Ordering::Relaxed) < QUEUE_BUDGET;
                    if pending.pointer_dirty && self.epoch > 0 && room {
                        pending.pointer_dirty = false;
                        let pointer = pending.pointer.as_ref().expect("a dirty pointer exists");
                        if !send(&self.out, &self.mailbox, &cursor_message(self.epoch, pointer)) {
                            return;
                        }
                    }
                    let room = self.mailbox.queued.load(Ordering::Relaxed) < QUEUE_BUDGET;
                    let ready = pending.hello.is_some() && pending.latest.is_some();
                    let due = self.next_due.is_none_or(|at| Instant::now() + self.slack() >= at);
                    let changed = !pending.dirty.is_empty() || pending.refresh || pending.settings_changed;
                    let now = Instant::now();
                    let motion_ends = self.in_video
                        && !self.heavy_at(now)
                        && pending.latest.as_ref().is_some_and(|s| !s.motion_at(now));
                    if room && ready && due && (changed || motion_ends) {
                        break;
                    }
                    if self.video_idle() {
                        drop_video = true;
                        break;
                    }
                    let wait = if ready && room && !due {
                        self.next_due
                            .map_or(Duration::ZERO, |at| at.saturating_duration_since(Instant::now() + self.slack()))
                    } else {
                        IDLE_WAIT
                    };
                    pending = mailbox.wake.wait_timeout(pending, wait).unwrap().0;
                }
                if drop_video {
                    None
                } else {
                    let snapshot = pending.latest.clone().expect("checked above");
                    let codecs = pending.hello.expect("checked above");
                    let reconfigure = self.epoch == 0 || std::mem::take(&mut pending.settings_changed);
                    let refresh = std::mem::take(&mut pending.refresh);
                    let dirty = pending.dirty.take(self.mailbox.width, self.mailbox.height);
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
            if !self.encode(&snapshot, refresh || reconfigure, dirty) {
                self.mailbox.close();
                return;
            }
        }
    }

    /// Picks the codec and starts a new epoch with its configuration.
    fn configure(&mut self, codecs: u8, settings: Settings) -> bool {
        self.settings = settings;
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
        // in AV1 again, the new epoch beginning with a full lossless refresh.
        let mut refresh = refresh;
        if !source_motion {
            self.lossless_over = false;
            self.lossless_times.clear();
            self.av1_times.clear();
            if self.av1_behind {
                self.av1_behind = false;
                if !self.configure(self.codecs, self.settings) {
                    return false;
                }
                refresh = true;
            }
        }
        self.revision += 1;
        // Motion tries lossless tiles first, until a frame does not fit the budget.
        let video_dirty = dirty.clone();
        if !source_motion || (!self.lossless_over && !self.in_video) {
            // Leaving video ends with a lossless refresh of the whole region at a newer revision.
            let full = refresh || self.in_video;
            let rects =
                if full { self.full_refresh(&snapshot.pixels) } else { self.sent.changed(&snapshot.pixels, dirty) };
            let rgb_limit = if source_motion { LOSSLESS_RGB_BYTES } else { usize::MAX };
            let messages = tiles::encode(
                &mut self.deflate,
                &snapshot.pixels,
                self.mailbox.width,
                &rects,
                self.epoch,
                self.revision,
                rgb_limit,
            );
            let bytes: usize = messages.iter().flatten().map(tile_bytes).sum();
            if let Some(messages) = messages.filter(|_| !source_motion || self.lossless_fits(bytes, started.elapsed()))
            {
                if !source_motion && !full && self.codec != Codec::None {
                    self.note_update(started, bytes, started.elapsed());
                }
                self.in_video = false;
                let (_, ok) = send_tiles(&self.out, &self.mailbox, &messages);
                return self.finish(started, bytes, ok);
            }
            // The tiles were recorded as sent: the video that replaces them covers them where it lies, they go as
            // tiles elsewhere (they count as changed now), and it ends with a full refresh.
            tracing::debug!(bytes, "motion leaves lossless tiles for video");
            self.activity.mark(&rects, started);
            self.unsent = rects;
            self.lossless_over = true;
        }
        self.encode_video(snapshot, refresh, started, video_dirty)
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
        self.lossless_times.push(took.as_micros() as u32);
        if self.lossless_times.len() < LOSSLESS_SAMPLES {
            return true;
        }
        !over_budget(std::mem::take(&mut self.lossless_times), self.frame_interval())
    }

    /// The video's bytes per frame: its bitrate over its frame rate.
    fn frame_bytes(&self) -> f64 {
        f64::from(self.settings.bitrate_kbps) * 1000.0 / 8.0 * self.frame_interval().as_secs_f64()
    }

    /// Every tile of the output, recorded as sent: a full lossless refresh.
    fn full_refresh(&mut self, pixels: &[u32]) -> Vec<Rect> {
        self.sent.record_all(pixels);
        region::tiles_outside(&Rect::new(0, 0, self.mailbox.width, self.mailbox.height), None)
    }

    fn finish(&mut self, started: Instant, bytes: usize, ok: bool) -> bool {
        self.next_due = Some(next_due(self.next_due, started, self.frame_interval()));
        self.record(started.elapsed(), bytes);
        ok
    }

    /// The region the video should take at `started`, if it should move: at the start of an episode, where things
    /// move for good, else where anything changed (anywhere, if the detector saw motion the tiles do not show); while
    /// it shows, a region that holds what moves for good outside the current one, or a smaller one once that has
    /// fitted in it for [`SHRINK_AFTER`] ([`SHRINK_AFTER_STILL`] when what it would leave is wholly still), at most once
    /// per [`REGION_MOVE_EVERY`]; a region that grew back within [`SHRINK_AFTER`] of shrinking (an overlay that comes
    /// and goes at its edge) shrinks only the slow way for the rest of the episode. Brief changes outside the region
    /// (a hover, a ticking counter) go as tiles and never move it.
    fn next_region(&mut self, started: Instant) -> Option<Rect> {
        let output = Rect::new(0, 0, self.mailbox.width, self.mailbox.height);
        let wanted = if self.codec == Codec::Av1 { self.activity.moving(started) } else { Some(output) };
        let current = self.region.filter(|_| self.video.is_some());
        let Some(current) = current.filter(|_| self.in_video) else {
            // An episode's first frames: what moves for good, else whatever changed.
            (self.shrunk_at, self.flapping) = (None, false);
            let wanted = wanted.or_else(|| self.activity.region(started)).unwrap_or(output);
            return (current != Some(wanted)).then_some(wanted);
        };
        let refit = self.rate_refit(started, &current);
        let Some(wanted) = wanted.filter(|w| *w != current) else {
            self.smaller_since = None;
            return refit.then_some(current);
        };
        let rested = self.region_moved_at.is_none_or(|t| started.duration_since(t) >= REGION_MOVE_EVERY);
        if !region::contains(&current, &wanted) {
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
        let cap = self.settings.max_fps.max(1);
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
        if from.is_none_or(|from| !region::contains(&from, next)) {
            self.video_sent.clear();
            self.video_sent_since = None;
        }
        let fitted = Some(self.video_fps).filter(|fps| *fps > 0 && showing);
        self.region_fps(now, next).or(fitted).unwrap_or(self.settings.max_fps)
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
        let unsent = std::mem::take(&mut self.unsent);
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
        let (Some(encoder), Some(region)) = (self.video.as_mut(), self.region) else {
            // Without an encoder, motion goes as a full lossless refresh, whatever its size.
            let rects = self.full_refresh(&snapshot.pixels);
            let messages = tiles::encode(
                &mut self.deflate,
                &snapshot.pixels,
                self.mailbox.width,
                &rects,
                self.epoch,
                self.revision,
                usize::MAX,
            )
            .expect("an unbounded encode always completes");
            self.in_video = false;
            let (bytes, ok) = send_tiles(&self.out, &self.mailbox, &messages);
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
        // Around the video, lossless tiles: what changed there, all of it on a refresh, and whatever the video left
        // when it moved (its old picture there, whatever the client was sent before). They take this revision and the
        // video the next, so a late frame of the old region never covers them.
        // A refresh's tiles hold everything else that changed or went unsent around it.
        let (mut rects, forced) = if refresh {
            let output = Rect::new(0, 0, self.mailbox.width, self.mailbox.height);
            (Vec::new(), region::tiles_outside(&output, Some(&region)))
        } else {
            let mut forced =
                moved_from.filter(|_| moved).map_or_else(Vec::new, |old| region::tiles_outside(&old, Some(&region)));
            // What a lossless trial left unsent, outside the video.
            forced.extend(unsent.into_iter().filter(|r| !region::contains(&region, r)));
            let around: Vec<Rect> = dirty.into_iter().filter(|r| !region::contains(&region, r)).collect();
            (self.sent.changed(&snapshot.pixels, around), forced)
        };
        self.sent.record(&snapshot.pixels, &forced);
        rects.extend(forced);
        let mut bytes = 0;
        let mut ok = true;
        let mut video_took = None;
        if !rects.is_empty() {
            let messages = tiles::encode(
                &mut self.deflate,
                &snapshot.pixels,
                self.mailbox.width,
                &rects,
                self.epoch,
                self.revision,
                usize::MAX,
            )
            .expect("an unbounded encode always completes");
            (bytes, ok) = send_tiles(&self.out, &self.mailbox, &messages);
            self.revision += 1;
        }
        if keyframe || inside {
            let (pixels, stride) =
                region_pixels(&snapshot.pixels, self.mailbox.width as usize, &region, &mut self.crop);
            let encode_started = Instant::now();
            let encoded = encoder.encode(pixels, stride, keyframe);
            video_took = Some(encode_started.elapsed());
            // Only frames with new damage of the region keep up with the source; a refresh or a keyframe alone does not.
            if inside {
                self.note_video_frame(started);
            }
            if keyframe {
                self.keyframe_at = Some(started);
            }
            match encoded {
                Ok(payload) if payload.len() > MAX_VIDEO_PAYLOAD => {
                    // Too large for one message: the region goes as lossless tiles, which split under the bound.
                    tracing::debug!(bytes = payload.len(), "a video frame over the message bound goes as tiles");
                    let rects = region::tiles_outside(&region, None);
                    self.sent.record(&snapshot.pixels, &rects);
                    let messages = tiles::encode(
                        &mut self.deflate,
                        &snapshot.pixels,
                        self.mailbox.width,
                        &rects,
                        self.epoch,
                        self.revision,
                        usize::MAX,
                    )
                    .expect("an unbounded encode always completes");
                    let (tile_bytes, tiles_ok) = send_tiles(&self.out, &self.mailbox, &messages);
                    (bytes, ok) = (bytes + tile_bytes, ok & tiles_ok);
                    self.keyframe_owed = true;
                }
                Ok(payload) if !payload.is_empty() => {
                    self.keyframe_owed &= !keyframe;
                    bytes += payload.len();
                    self.stats.video_frames += 1;
                    let rect =
                        WireRect { x: region.x as u16, y: region.y as u16, w: region.w as u16, h: region.h as u16 };
                    ok &= send(
                        &self.out,
                        &self.mailbox,
                        &Message::Video {
                            epoch: self.epoch,
                            revision: self.revision,
                            rect,
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
        self.av1_times.push(took.as_micros() as u32);
        if self.av1_times.len() < AV1_SAMPLES {
            return false;
        }
        over_budget(std::mem::take(&mut self.av1_times), self.frame_interval())
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
    use crate::stream::viewer::{Out, Settings};
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
        let mailbox = Arc::new(Mailbox::new(width, height));
        let (out, frames) = std::sync::mpsc::channel();
        let mut encoder = Encoder::new(mailbox, out);
        encoder.codec = Codec::Av1;
        let now = Instant::now();
        for ms in (0..=600u16).step_by(40).rev() {
            encoder.activity.mark(&[CLIP], now - Duration::from_millis(ms.into()));
        }
        let mut seed = 0x2545_f491_u32;
        let mut pixels = vec![0u32; (width * height) as usize];
        for y in 0..height {
            for x in 256..width {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                pixels[(y * width + x) as usize] = seed | 0xff00_0000;
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

    const CLIP: Rect = Rect { x: 0, y: 0, w: 64, h: 64 };
    const NOISE: Rect = Rect { x: 256, y: 0, w: 256, h: 256 };

    /// How many times tiles in `frames` covered each pixel of a 512x256 output.
    fn tile_coverage(frames: &std::sync::mpsc::Receiver<Out>) -> Vec<u8> {
        let mut covered = vec![0u8; 512 * 256];
        for out in frames.try_iter() {
            let Out::Now(frame) = out else { continue };
            let Ok(Link::Relay(Message::Tiles { rects, .. })) = Link::decode(&frame[4..], Direction::ServerToClient)
            else {
                continue;
            };
            for r in rects {
                for y in r.y..r.y + r.h {
                    for x in r.x..r.x + r.w {
                        covered[y as usize * 512 + x as usize] += 1;
                    }
                }
            }
        }
        covered
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
        let mailbox = Arc::new(Mailbox::new(256, 256));
        let (out, _frames) = std::sync::mpsc::channel();
        let mut encoder = Encoder::new(mailbox.clone(), out);
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
        let lagging = Arc::new(Mailbox::new(256, 256));
        let mut behind = Encoder::new(lagging.clone(), std::sync::mpsc::channel().0);
        composed(&mut lagging.pending.lock().unwrap().compositions, start, (20_000, 23_000), 33, region);
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
        let mailbox = Arc::new(Mailbox::new(256, 256));
        let mut encoder = Encoder::new(mailbox.clone(), std::sync::mpsc::channel().0);
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let region = Rect::new(0, 0, 64, 64);
        // 40 frames a second, all of them sent.
        composed(&mut mailbox.pending.lock().unwrap().compositions, start, (0, 2000), 25, region);
        (0..2000).step_by(25).for_each(|ms| encoder.note_video_frame(at(ms)));
        encoder.video_fps = VIDEO_FPS;
        assert_eq!(encoder.region_fps(at(2000), &region), Some(VIDEO_FPS));
        encoder.video_fps = 60;
        assert_eq!(encoder.region_fps(at(2000), &region), Some(60));
    }

    #[test]
    fn a_region_that_grows_back_soon_after_shrinking_shrinks_only_the_slow_way() {
        let mailbox = Arc::new(Mailbox::new(1280, 800));
        let mut encoder = Encoder::new(mailbox, std::sync::mpsc::channel().0);
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
        let mailbox = Arc::new(Mailbox::new(256, 256));
        let mut encoder = Encoder::new(mailbox.clone(), std::sync::mpsc::channel().0);
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let (a, b) = (Rect::new(0, 0, 64, 64), Rect::new(128, 128, 64, 64));
        let grown = Rect::new(64, 64, 192, 192);
        // A 30 fps source on B for two seconds; 30 frames a second were sent, for A.
        composed(&mut mailbox.pending.lock().unwrap().compositions, start, (0, 2000), 33, b);
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
        let mailbox = Arc::new(Mailbox::new(256, 256));
        let (out, _frames) = std::sync::mpsc::channel();
        let mut encoder = Encoder::new(mailbox, out);
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
        let mailbox = Arc::new(Mailbox::new(256, 256));
        let (out, frames) = std::sync::mpsc::channel();
        let mut encoder = Encoder::new(mailbox, out);
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

    /// A frame of noise over a large output, at the highest bitrate and the lowest frame rate the settings allow: a
    /// budget per frame well past the message bound.
    #[test]
    fn every_message_of_a_costly_video_frame_fits_the_bound() {
        let (width, height) = (1920, 1080);
        let mailbox = Arc::new(Mailbox::new(width, height));
        let (out, frames) = std::sync::mpsc::channel();
        let mut encoder = Encoder::new(mailbox, out);
        encoder.codec = Codec::Av1;
        encoder.settings = Settings {
            codec: CodecChoice::Av1,
            bitrate_kbps: *display_protocol::frame::BITRATE_KBPS.end(),
            max_fps: *display_protocol::frame::MAX_FPS.start(),
        };
        let mut seed = 0x2545_f491_u32;
        let pixels = (0..width * height)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed | 0xff00_0000
            })
            .collect();
        let snapshot = Snapshot { time_us: 0, pixels: Arc::new(pixels), damage: Vec::new(), motion_until: None };
        let output = Rect::new(0, 0, width, height);
        let now = Instant::now();
        encoder.activity.mark(&[output], now);
        assert!(encoder.encode_video(&snapshot, false, now, vec![output]));
        let mut covered = vec![false; (width * height) as usize];
        for out in frames.try_iter() {
            let Out::Now(frame) = out else { continue };
            let message = Link::decode(&frame[4..], Direction::ServerToClient).expect("a message within the bound");
            let rects = match message {
                Link::Relay(Message::Tiles { rects, .. }) => rects,
                Link::Relay(Message::Video { rect, .. }) => vec![rect],
                _ => continue,
            };
            for r in rects {
                for y in r.y..r.y + r.h {
                    covered[y as usize * width as usize + r.x as usize..][..r.w as usize].fill(true);
                }
            }
        }
        assert!(covered.iter().all(|c| *c), "the frame reached the viewer whole");
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
