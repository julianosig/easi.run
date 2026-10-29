//! A recording's worker: the composed screen at a steady [`FPS`], with the pointer and the person's camera where they
//! put it, and one sound track mixing the page's sound with their microphone, encoded at a higher rate than the live
//! stream (AV1 and Opus) and written as WebM to a socket the display offers as a download while it grows (see
//! `recording`).
//!
//! It runs on a thread of its own at a lower priority than the live stream. Its inputs come through a bounded queue
//! (the newest screen and pointer through slots, the stop through a flag), so a burst is dropped rather than queued
//! without bound, and every input waiting is taken before each frame, however slow encoding gets. Time is the
//! display's clock from the moment it started: a frame every 1/[`FPS`] s (frames it falls behind on are skipped), and
//! sound in 20 ms steps written [`AUDIO_DELAY_US`] late so pieces arriving late (the microphone's, over the network)
//! still join; a step nothing arrived for is silence. A microphone packet stamped before the recording, and a camera
//! picture or microphone packet stamped too far ahead, are dropped.

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, sync_channel};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use display_protocol::{AUDIO_FRAME_US, AUDIO_SAMPLE_RATE, CameraLayout, LAYOUT_UNITS};

use super::av1::Av1;
use super::opus::{Decoder, FRAME_PCM, FRAME_SAMPLES, Opus};
use super::webm::Webm;

pub const FPS: u8 = 30;
/// The video rate: four times the live stream's default, for a picture close to the screen itself.
const VIDEO_KBPS: u32 = 10_000;
/// A keyframe at least every two seconds, so a player can start (or seek) close to any moment.
const KEYFRAME_EVERY: u64 = 2 * FPS as u64;
/// Frames encoded again from the same picture before a still one is left out (until it changes or a keyframe is due),
/// so the encoder sharpens it first: encoding a still 1280x800 screen costs some 7 ms a frame.
const STILL_FRAMES: u32 = 15;
const AUDIO_BITRATE: i32 = 128_000;
/// How late sound is written, so late pieces still join it.
pub const AUDIO_DELAY_US: u64 = 400_000;
/// A picture or packet stamped further ahead than this is dropped.
const MAX_AHEAD_US: u64 = 2_000_000;
/// Sound steps kept for mixing at once: well past the delay above.
const MAX_PENDING_STEPS: usize = 256;
/// A recording ends past these.
pub const MAX_SECONDS: u64 = 60 * 60;
pub const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// Camera pictures closer together than this are dropped undecoded (the viewer sends 15 a second).
const CAMERA_EVERY: Duration = Duration::from_millis(50);
/// A camera that sent nothing for this long is no longer drawn.
const CAMERA_STALE: Duration = Duration::from_secs(3);
/// Inputs waiting for the worker.
const QUEUE: usize = 128;
/// A write to the download that makes no progress for this long means nobody is receiving it: the recording fails.
const WRITE_TIMEOUT: Duration = Duration::from_secs(20);
/// Once stopped, the rest of the file must be taken within this, or the recording fails (the viewer waits 15 s).
const FINISH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one blocked write waits before the worker checks its deadlines again.
const WRITE_STEP: Duration = Duration::from_millis(250);
/// The worker's niceness: below the compositor and the live encoders.
const NICENESS: i32 = 10;

pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub layout: CameraLayout,
}

pub enum Input {
    /// The page's sound: 20 ms of interleaved stereo starting at `timestamp_us`. Only a recording that asked for it
    /// gets it (the audio thread's tap is set for it alone, see `crate::recording`).
    Sound {
        timestamp_us: u64,
        pcm: Vec<i16>,
    },
    Camera {
        timestamp_us: u64,
        jpeg: Vec<u8>,
    },
    Microphone {
        timestamp_us: u64,
        packet: Vec<u8>,
    },
}

/// The pointer's picture: premultiplied ARGB, with the point it marks.
pub struct Picture {
    width: usize,
    height: usize,
    hot: (usize, usize),
    pixels: Vec<u32>,
}

impl Picture {
    /// The picture over `frame`, `width` pixels a row, its point at `(x, y)`, cut at the frame's edges.
    pub fn draw(&self, frame: &mut [u32], width: usize, (x, y): (i64, i64)) {
        let height = frame.len() / width;
        let left = x - self.hot.0 as i64;
        let top = y - self.hot.1 as i64;
        for py in 0..self.height {
            let sy = top + py as i64;
            if sy < 0 || sy >= height as i64 {
                continue;
            }
            for px in 0..self.width {
                let sx = left + px as i64;
                if sx < 0 || sx >= width as i64 {
                    continue;
                }
                let dst = &mut frame[sy as usize * width + sx as usize];
                *dst = crate::compose::blend(self.pixels[py * self.width + px], *dst);
            }
        }
    }
}

/// The pointer as the recording draws it: where it is, and its picture (none while hidden).
#[derive(Clone, Default)]
struct Sprite {
    at: (u16, u16),
    picture: Option<Arc<Picture>>,
}

/// The pointer's picture for a viewer's pointer: hidden, a custom image (QOI, straight alpha), or, for any named shape,
/// an arrow (the named shapes are drawn by each viewer's own browser, so the display has no picture of them).
pub(super) fn picture(pointer: &super::cursor::Pointer) -> Option<Arc<Picture>> {
    match pointer.shape {
        0 => None,
        super::cursor::IMAGE => {
            let (header, rgba) = qoi::decode_to_vec(&*pointer.image).ok()?;
            let channels = header.channels.as_u8() as usize;
            let (width, height) = (header.width as usize, header.height as usize);
            if channels != 4 || rgba.len() != width * height * 4 {
                return Some(ARROW.clone());
            }
            let pixels = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| {
                    let a = u32::from(p[3]);
                    let pre = |c: u8| (u32::from(c) * a + 127) / 255;
                    a << 24 | pre(p[0]) << 16 | pre(p[1]) << 8 | pre(p[2])
                })
                .collect();
            let hot = (usize::from(pointer.hot_x).min(width - 1), usize::from(pointer.hot_y).min(height - 1));
            Some(Arc::new(Picture { width, height, hot, pixels }))
        }
        _ => Some(ARROW.clone()),
    }
}

/// The arrow: a black outline (`X`) around white (`.`), pointing at its top-left corner.
static ARROW: LazyLock<Arc<Picture>> = LazyLock::new(|| {
    const ROWS: [&str; 19] = [
        "X           ",
        "XX          ",
        "X.X         ",
        "X..X        ",
        "X...X       ",
        "X....X      ",
        "X.....X     ",
        "X......X    ",
        "X.......X   ",
        "X........X  ",
        "X.........X ",
        "X......XXXXX",
        "X...X..X    ",
        "X..XX..X    ",
        "X.X  X..X   ",
        "XX   X..X   ",
        "X     X..X  ",
        "      X..X  ",
        "       XX   ",
    ];
    let pixels = ROWS
        .iter()
        .flat_map(|row| {
            row.bytes().map(|b| match b {
                b'X' => 0xff00_0000,
                b'.' => 0xffff_ffff,
                _ => 0,
            })
        })
        .collect();
    Arc::new(Picture { width: ROWS[0].len(), height: ROWS.len(), hot: (0, 0), pixels })
});

/// How a recording ended, with what it wrote.
pub struct Finished {
    pub failure: Option<String>,
    pub bytes: u64,
    pub seconds: u32,
}

/// The handle the compositor keeps: the queue in, the slots the newest screen and pointer go in, and the stop.
pub struct Worker {
    inputs: SyncSender<Input>,
    shared: Arc<Shared>,
}

/// What the worker shares with the compositor.
struct Shared {
    latest: Mutex<Arc<Vec<u32>>>,
    pointer: Mutex<Sprite>,
    /// A new camera layout, until the next frame takes it.
    layout: Mutex<Option<CameraLayout>>,
    stop: AtomicBool,
}

impl Worker {
    /// Starts recording `first` (the screen now, with the pointer at `at` showing `picture`) and what follows, into
    /// `sink`; `done` hears the end, from the worker's thread.
    pub fn start(
        settings: Settings,
        first: Arc<Vec<u32>>,
        (at, picture): ((u16, u16), Option<Arc<Picture>>),
        sink: UnixStream,
        done: impl FnOnce(Finished) + Send + 'static,
    ) -> Result<Worker, String> {
        sink.set_write_timeout(Some(WRITE_STEP)).map_err(|e| e.to_string())?;
        let (inputs, receiver) = sync_channel(QUEUE);
        let shared = Arc::new(Shared {
            latest: Mutex::new(first),
            pointer: Mutex::new(Sprite { at, picture }),
            layout: Mutex::new(None),
            stop: AtomicBool::new(false),
        });
        let worker = Worker { inputs, shared: shared.clone() };
        std::thread::Builder::new()
            .name("recording".into())
            .spawn(move || {
                // Below the live stream: a recording may fall behind, a viewer may not.
                let _ = rustix::process::setpriority_process(None, NICENESS);
                done(run(&settings, &shared, &receiver, sink));
            })
            .map_err(|e| format!("the recording thread: {e}"))?;
        Ok(worker)
    }

    /// Ends the recording: the worker finishes the file within a frame, or as soon as its write in progress allows.
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }

    /// The pointer moved to `at`.
    pub fn pointer_at(&self, at: (u16, u16)) {
        self.shared.pointer.lock().unwrap_or_else(PoisonError::into_inner).at = at;
    }

    /// The camera moved or changed its look: the next frame draws it so (a newer layout replaces one not yet drawn).
    pub fn layout(&self, layout: CameraLayout) {
        *self.shared.layout.lock().unwrap_or_else(PoisonError::into_inner) = Some(layout);
    }

    /// The pointer's picture changed (none: hidden).
    pub fn pointer_picture(&self, picture: Option<Arc<Picture>>) {
        self.shared.pointer.lock().unwrap_or_else(PoisonError::into_inner).picture = picture;
    }

    /// Hands the worker an input; false when it is gone or behind (the input is dropped).
    pub fn send(&self, input: Input) -> bool {
        self.inputs.try_send(input).is_ok()
    }

    /// The queue in, for the audio thread's samples.
    pub fn sender(&self) -> SyncSender<Input> {
        self.inputs.clone()
    }

    /// The screen changed: the next frame takes this one.
    pub fn frame(&self, pixels: Arc<Vec<u32>>) {
        *self.shared.latest.lock().unwrap_or_else(PoisonError::into_inner) = pixels;
    }
}

fn run(settings: &Settings, shared: &Shared, inputs: &Receiver<Input>, sink: UnixStream) -> Finished {
    let start_us = super::clock_us();
    let mut state = match Recorder::new(settings, start_us) {
        Ok(state) => state,
        Err(why) => return Finished { failure: Some(why), bytes: 0, seconds: 0 },
    };
    let mut out = Out { sink, bytes: 0, stop: &shared.stop, stopped_at: None };
    let frame_us = 1_000_000 / u64::from(FPS);
    // Frame n is due at its exact time (n seconds over FPS), as its timestamp says, so video never drifts from the
    // sound's real clock (a truncated frame length would, by a third of a microsecond a frame).
    let due_at = |frame: u64| start_us + frame * 1_000_000 / u64::from(FPS);
    let mut frame: u64 = 0;
    let mut failure = None;
    while !shared.stop.load(Ordering::Relaxed) {
        let now = super::clock_us();
        let due = due_at(frame);
        if now < due {
            match inputs.recv_timeout(Duration::from_micros(due - now)) {
                Ok(input) => state.take(input, now),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            continue;
        }
        // Everything waiting first, however late the frame: a slow encoder must not starve the sound or the camera.
        if !drain(&mut state, inputs, now) {
            break;
        }
        // Behind by more than a frame: the frames missed are skipped, not caught up.
        if now - due > 2 * frame_us {
            frame = (now - start_us) * u64::from(FPS) / 1_000_000;
        }
        let pixels = shared.latest.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let pointer = shared.pointer.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if let Some(layout) = shared.layout.lock().unwrap_or_else(PoisonError::into_inner).take() {
            state.set_layout(layout);
        }
        let made = state.video(&pixels, &pointer, frame).and_then(|mut made| {
            made.extend(state.sound_until(now.saturating_sub(AUDIO_DELAY_US))?);
            Ok(made)
        });
        frame += 1;
        if let Err(why) = made.and_then(|made| out.write(&made)) {
            failure = Some(why);
            break;
        }
        if out.bytes >= MAX_BYTES || (now - start_us) / 1_000_000 >= MAX_SECONDS {
            break;
        }
    }
    // The end: the sound to the last frame's end (with what came in before the stop), then the clusters still open.
    let end_us = due_at(frame);
    // Stopped before its first keyframe: nothing started the file, so there is none to offer.
    if failure.is_none() && state.keyframe_at.is_none() {
        failure = Some("the recording stopped before its first frame".into());
    }
    if failure.is_none() {
        drain(&mut state, inputs, super::clock_us());
        let tail = state.sound_end(end_us).map(|mut made| {
            made.extend(state.webm.finish());
            made
        });
        if let Err(why) = tail.and_then(|made| out.write(&made)) {
            failure = Some(why);
        }
    }
    let seconds = ((end_us - start_us) / 1_000_000) as u32;
    Finished { failure, bytes: out.bytes, seconds }
}

/// Takes the inputs waiting (at most a queue's worth); false once the queue is gone.
fn drain(state: &mut Recorder, inputs: &Receiver<Input>, now: u64) -> bool {
    for _ in 0..QUEUE {
        match inputs.try_recv() {
            Ok(input) => state.take(input, now),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => return false,
        }
    }
    true
}

/// The download the file goes to, with what was written.
struct Out<'a> {
    sink: UnixStream,
    bytes: u64,
    stop: &'a AtomicBool,
    /// When the worker first saw the stop while writing (the file must then be finished soon).
    stopped_at: Option<Instant>,
}

impl Out<'_> {
    /// Writes `data` whole, or fails when nobody takes it for [`WRITE_TIMEOUT`], or (stopped) within
    /// [`FINISH_TIMEOUT`].
    fn write(&mut self, mut data: &[u8]) -> Result<(), String> {
        let mut progress = Instant::now();
        while !data.is_empty() {
            match self.sink.write(data) {
                Ok(0) => return Err("the download closed".into()),
                Ok(n) => {
                    data = &data[n..];
                    self.bytes += n as u64;
                    progress = Instant::now();
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("the download stopped taking the recording: {e}")),
            }
            if self.stop.load(Ordering::Relaxed) {
                let stopped = *self.stopped_at.get_or_insert_with(Instant::now);
                if stopped.elapsed() > FINISH_TIMEOUT {
                    return Err("the download did not take the end of the recording in time".into());
                }
            }
            if progress.elapsed() > WRITE_TIMEOUT {
                return Err("the download stopped taking the recording".into());
            }
        }
        Ok(())
    }
}

/// Where the camera goes on a `width` by `height` screen, as `(x, y, w, h)`: a square of the side asked (no taller
/// than the screen), placed as asked and kept inside, the same square the person's preview shows
/// (`client/src/recorder.ts`).
pub fn camera_rect(width: u32, height: u32, layout: &CameraLayout) -> (usize, usize, usize, usize) {
    let (fw, fh) = (f64::from(width), f64::from(height));
    let units = f64::from(LAYOUT_UNITS);
    let side = (f64::from(layout.side) * fw / units).round().min(fh);
    let x = (f64::from(layout.x) * fw / units).round().clamp(0.0, (fw - side).max(0.0));
    let y = (f64::from(layout.y) * fh / units).round().clamp(0.0, (fh - side).max(0.0));
    (x as usize, y as usize, side as usize, side as usize)
}

/// The last camera picture, as decoded, and when it came.
struct Photo {
    rgb: Vec<u8>,
    width: usize,
    height: usize,
    at: Instant,
}

/// The camera, ready for the frame: its picture scaled to the card, and the card's mask (how much of each pixel is
/// the card), both `w` by `h`.
struct Card {
    pixels: Vec<u32>,
    mask: Vec<u8>,
    /// What the mask was made for (side, radius): a new picture of the same card reuses it.
    shaped: (usize, u8),
}

/// Everything a frame is drawn from, compared by identity. The screen is held, so its buffer is never painted again
/// while compared.
struct Shown {
    screen: Arc<Vec<u32>>,
    pointer: Option<((u16, u16), Arc<Picture>)>,
    /// The card's version, while it is drawn.
    card: Option<u64>,
}

impl PartialEq for Shown {
    fn eq(&self, other: &Shown) -> bool {
        let pointers = match (&self.pointer, &other.pointer) {
            (Some((a, p)), Some((b, q))) => a == b && Arc::ptr_eq(p, q),
            (a, b) => a.is_none() && b.is_none(),
        };
        Arc::ptr_eq(&self.screen, &other.screen) && pointers && self.card == other.card
    }
}

struct Recorder {
    width: usize,
    height: usize,
    rect: (usize, usize, usize, usize),
    layout: CameraLayout,
    start_us: u64,
    video: Av1,
    sound: Opus,
    /// The encoder's lookahead, in 48 kHz samples.
    pre_skip: u16,
    microphone: Decoder,
    webm: Webm,
    photo: Option<Photo>,
    card: Option<Card>,
    /// When the last camera picture was decoded.
    camera_at: Option<Instant>,
    /// The last keyframe's number.
    keyframe_at: Option<u64>,
    /// What the last frame encoded showed, and how many encoded since showed the same.
    shown: Option<Shown>,
    same: u32,
    /// Moved on as the camera's card changes.
    card_version: u64,
    work: Vec<u32>,
    /// Sound steps being mixed, by number (20 ms each from the start), and the next to write.
    steps: BTreeMap<u64, Vec<i32>>,
    next_step: u64,
}

impl Recorder {
    fn new(settings: &Settings, start_us: u64) -> Result<Recorder, String> {
        let sound = Opus::with_bitrate(AUDIO_BITRATE)?;
        let pre_skip = sound.lookahead()?;
        Ok(Recorder {
            width: settings.width as usize,
            height: settings.height as usize,
            rect: camera_rect(settings.width, settings.height, &settings.layout),
            layout: settings.layout,
            start_us,
            video: Av1::new(settings.width, settings.height, VIDEO_KBPS, FPS)?,
            sound,
            pre_skip,
            microphone: Decoder::new()?,
            webm: Webm::new(settings.width, settings.height, pre_skip),
            photo: None,
            card: None,
            camera_at: None,
            keyframe_at: None,
            shown: None,
            same: 0,
            card_version: 0,
            work: vec![0; settings.width as usize * settings.height as usize],
            steps: BTreeMap::new(),
            next_step: 0,
        })
    }

    /// One input from the queue, at `now`.
    fn take(&mut self, input: Input, now: u64) {
        match input {
            Input::Sound { timestamp_us, pcm } => self.mix(timestamp_us, &pcm, now),
            Input::Microphone { timestamp_us, packet } => {
                if let Ok(pcm) = self.microphone.decode(&packet) {
                    self.mix(timestamp_us, &pcm, now);
                }
            }
            Input::Camera { timestamp_us, jpeg } => {
                let soon = self.camera_at.is_some_and(|at| at.elapsed() < CAMERA_EVERY);
                if timestamp_us <= now + MAX_AHEAD_US && !soon {
                    self.camera_at = Some(Instant::now());
                    self.camera(&jpeg);
                }
            }
        }
    }

    /// Frame number `frame`, with the camera and the pointer on it; returns the file's bytes it made ready. The first
    /// frame encoded is a keyframe, and so is the first encoded [`KEYFRAME_EVERY`] frames or more after the last one
    /// (frames may be skipped). A frame
    /// the same as the last [`STILL_FRAMES`] encoded is left out: the player shows the last one until the next.
    fn video(&mut self, screen: &Arc<Vec<u32>>, pointer: &Sprite, frame: u64) -> Result<Vec<u8>, String> {
        let pixels = screen.as_slice();
        if pixels.len() != self.width * self.height {
            return Err("the screen changed size during the recording".into());
        }
        let keyframe = self.keyframe_at.is_none_or(|k| frame >= k + KEYFRAME_EVERY);
        let fresh = self.photo.as_ref().is_some_and(|p| p.at.elapsed() < CAMERA_STALE) && self.card.is_some();
        let shown = Shown {
            screen: screen.clone(),
            pointer: pointer.picture.clone().map(|picture| (pointer.at, picture)),
            card: fresh.then_some(self.card_version),
        };
        // A keyframe due is encoded whatever it shows, and counts as the first of a still picture's.
        if keyframe {
            self.same = 0;
            self.shown = Some(shown);
        } else if self.still(shown) {
            return Ok(Vec::new());
        }
        if keyframe {
            self.keyframe_at = Some(frame);
        }
        let unit = if fresh || pointer.picture.is_some() {
            self.work.copy_from_slice(pixels);
            if fresh {
                self.draw_card();
            }
            if let Some(picture) = &pointer.picture {
                picture.draw(&mut self.work, self.width, (pointer.at.0.into(), pointer.at.1.into()));
            }
            self.video.encode(&self.work, self.width, keyframe)?
        } else {
            self.video.encode(pixels, self.width, keyframe)?
        };
        let ms = frame * 1000 / u64::from(FPS);
        Ok(self.webm.video(ms, keyframe, &unit))
    }

    /// Whether a frame showing `shown` is the same as the last [`STILL_FRAMES`] encoded, so it can be left out; if not,
    /// it is the one encoded next.
    fn still(&mut self, shown: Shown) -> bool {
        let same = self.shown.as_ref() == Some(&shown);
        if same && self.same >= STILL_FRAMES {
            return true;
        }
        self.same = if same { self.same + 1 } else { 0 };
        self.shown = Some(shown);
        false
    }

    /// The camera over the frame: the picture inside, the screen past its rounded edge.
    fn draw_card(&mut self) {
        let Some(card) = &self.card else { return };
        let (x0, y0, w, h) = self.rect;
        for y in 0..h {
            let row = (y0 + y) * self.width + x0;
            for x in 0..w {
                let i = y * w + x;
                let dst = &mut self.work[row + x];
                match card.mask[i] {
                    0 => {}
                    255 => *dst = card.pixels[i],
                    part => *dst = blend(*dst, card.pixels[i], part),
                }
            }
        }
    }

    /// The camera moved or changed its look: the card is made again from the last picture, which keeps its age.
    fn set_layout(&mut self, layout: CameraLayout) {
        self.layout = layout;
        self.rect = camera_rect(self.width as u32, self.height as u32, &layout);
        self.make_card();
    }

    /// A new camera picture, decoded; the card is made from it.
    fn camera(&mut self, jpeg: &[u8]) {
        let Some(super::camera::Rgb { rgb, width, height }) = super::camera::decode(jpeg) else { return };
        self.photo = Some(Photo { rgb, width, height, at: Instant::now() });
        self.make_card();
    }

    /// The card from the last picture: its largest centred square, scaled to the card (covering it, as the preview
    /// does), and the card's shape.
    fn make_card(&mut self) {
        let Some(photo) = &self.photo else { return };
        self.card_version += 1;
        let (_, _, w, h) = self.rect;
        let (sw, sh) = (photo.width, photo.height);
        let scale = (w as f64 / sw as f64).max(h as f64 / sh as f64);
        let (cw, ch) = (w as f64 / scale, h as f64 / scale);
        let (cx, cy) = ((sw as f64 - cw) / 2.0, (sh as f64 - ch) / 2.0);
        let mut pixels = vec![0u32; w * h];
        // A pixel's source column depends on its column alone: worked out once, not again on every row.
        let columns: Vec<usize> = (0..w).map(|x| ((cx + (x as f64 + 0.5) / scale) as usize).min(sw - 1)).collect();
        for y in 0..h {
            let sy = ((cy + (y as f64 + 0.5) / scale) as usize).min(sh - 1);
            for (x, &sx) in columns.iter().enumerate() {
                let p = (sy * sw + sx) * 3;
                let rgb = &photo.rgb;
                pixels[y * w + x] =
                    0xff00_0000 | u32::from(rgb[p]) << 16 | u32::from(rgb[p + 1]) << 8 | u32::from(rgb[p + 2]);
            }
        }
        let shaped = (w, self.layout.radius);
        let mask = match self.card.take() {
            Some(card) if card.shaped == shaped => card.mask,
            _ => card_mask(w, shaped.1),
        };
        self.card = Some(Card { pixels, mask, shaped });
    }

    /// Adds sound starting at `timestamp_us` (display clock) to its 20 ms step.
    fn mix(&mut self, timestamp_us: u64, pcm: &[i16], now: u64) {
        if timestamp_us < self.start_us || timestamp_us > now + MAX_AHEAD_US {
            return;
        }
        let step = (timestamp_us - self.start_us + AUDIO_FRAME_US / 2) / AUDIO_FRAME_US;
        if step < self.next_step || (self.steps.len() >= MAX_PENDING_STEPS && !self.steps.contains_key(&step)) {
            return;
        }
        let mixed = self.steps.entry(step).or_insert_with(|| vec![0; FRAME_PCM]);
        for (m, &s) in mixed.iter_mut().zip(pcm) {
            *m = m.saturating_add(i32::from(s));
        }
    }

    /// Writes every sound step that ended by `until_us`, silence where nothing arrived.
    fn sound_until(&mut self, until_us: u64) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        while self.start_us + (self.next_step + 1) * AUDIO_FRAME_US <= until_us {
            let (ms, packet) = self.encode_step()?;
            out.extend(self.webm.audio(ms, &packet));
        }
        Ok(out)
    }

    /// Writes the sound to `end_us`: the steps that ended by it, then the one it ends in and as many more as carry its
    /// last samples out of the encoder (which holds back its lookahead), the samples past `end_us` marked for the
    /// player to drop.
    fn sound_end(&mut self, end_us: u64) -> Result<Vec<u8>, String> {
        let mut out = self.sound_until(end_us)?;
        let rest_us = end_us.saturating_sub(self.start_us + self.next_step * AUDIO_FRAME_US);
        let owed = rest_us * u64::from(AUDIO_SAMPLE_RATE) / 1_000_000 + u64::from(self.pre_skip);
        let steps = owed.div_ceil(FRAME_SAMPLES as u64);
        let discard = (steps * FRAME_SAMPLES as u64 - owed) as u16;
        for step in 1..=steps {
            let (ms, packet) = self.encode_step()?;
            out.extend(self.webm.audio_trimmed(ms, &packet, if step == steps { discard } else { 0 }));
        }
        Ok(out)
    }

    /// The next sound step encoded (silence where nothing arrived), with its time in milliseconds.
    fn encode_step(&mut self) -> Result<(u64, Vec<u8>), String> {
        let mut pcm = [0i16; FRAME_PCM];
        if let Some(mixed) = self.steps.remove(&self.next_step) {
            for (p, m) in pcm.iter_mut().zip(mixed) {
                *p = m.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            }
        }
        let packet = self.sound.encode(&pcm)?;
        let ms = self.next_step * AUDIO_FRAME_US / 1000;
        self.next_step += 1;
        Ok((ms, packet))
    }
}

/// `over` laid on `under` at `part` (0 to 255) of full strength.
fn blend(under: u32, over: u32, part: u8) -> u32 {
    let (a, b) = (u32::from(part), 255 - u32::from(part));
    let channel = |shift: u32| ((((over >> shift) & 0xff) * a + ((under >> shift) & 0xff) * b + 127) / 255) << shift;
    0xff00_0000 | channel(16) | channel(8) | channel(0)
}

/// The card's shape, a square of `side`, smoothed at its edges: for each pixel, how much of it is the card. Its corners
/// are rounded by `radius` percent of half its side: 0 is square, 100 a circle.
fn card_mask(side: usize, radius: u8) -> Vec<u8> {
    let size = side as f64;
    let radius = f64::from(radius.min(100)) / 100.0 * size / 2.0;
    let part = |d: f64| ((d + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut mask = vec![0; side * side];
    for y in 0..side {
        for x in 0..side {
            // Distance inside the rounded square's edge (negative outside).
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let dx = (radius - px).max(px - (size - radius)).max(0.0);
            let dy = (radius - py).max(py - (size - radius)).max(0.0);
            let inside = if dx > 0.0 && dy > 0.0 {
                radius - (dx * dx + dy * dy).sqrt()
            } else {
                px.min(py).min(size - px).min(size - py)
            };
            mask[y * side + x] = part(inside);
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUT: CameraLayout = CameraLayout { x: 7_400, y: 5_800, side: 2_400, radius: 24 };

    #[test]
    fn the_camera_sits_where_the_preview_shows_it() {
        // The same squares as client/test/recorder.test.ts.
        assert_eq!(camera_rect(1280, 800, &LAYOUT), (947, 464, 307, 307));
        let corner = CameraLayout { x: 10_000, y: 10_000, side: 4_000, ..LAYOUT };
        assert_eq!(camera_rect(1280, 800, &corner), (768, 288, 512, 512), "kept inside the screen");
        assert_eq!(camera_rect(1280, 400, &corner), (880, 0, 400, 400), "no taller than the screen");
        assert_eq!(camera_rect(1280, 800, &CameraLayout { x: 0, y: 0, side: 1_000, ..LAYOUT }), (0, 0, 128, 128));
    }

    #[test]
    fn cards_have_their_shape_and_smooth_edges() {
        let rounded = card_mask(100, 24);
        assert_eq!(rounded[0], 0, "a cut corner");
        assert_eq!(rounded[50 * 100], 255, "the picture to the left edge");
        assert_eq!(rounded[50 * 100 + 50], 255, "the middle is the picture");
        let square = card_mask(100, 0);
        assert_eq!(square[0], 255, "a square's corner is the picture");
        let circle = card_mask(100, 100);
        assert_eq!(circle[10 * 100 + 10], 0, "outside the circle");
        let edge = circle[14 * 100 + 14];
        assert!(edge > 0 && edge < 255, "an edge pixel is partly the card");
        assert_eq!(circle[50 * 100 + 50], 255);
        assert_eq!(blend(0xff00_0000, 0xffff_ffff, 128), 0xff80_8080);
    }

    fn recorder() -> Recorder {
        let settings = Settings { width: 320, height: 240, layout: CameraLayout { side: 1_600, ..LAYOUT } };
        Recorder::new(&settings, 1_000_000).unwrap()
    }

    #[test]
    fn a_recording_stopped_before_its_first_frame_fails_with_nothing_written() {
        let settings = Settings { width: 320, height: 240, layout: CameraLayout { side: 1_600, ..LAYOUT } };
        let shared = Shared {
            latest: Mutex::new(Arc::new(vec![0; 320 * 240])),
            pointer: Mutex::new(Sprite::default()),
            layout: Mutex::new(None),
            stop: AtomicBool::new(true),
        };
        let (_inputs, receiver) = sync_channel(QUEUE);
        let (sink, _download) = UnixStream::pair().unwrap();
        let finished = run(&settings, &shared, &receiver, sink);
        assert!(finished.failure.is_some(), "no keyframe started the file");
        assert_eq!(finished.bytes, 0);
    }

    #[test]
    fn sound_steps_mix_in_time_and_a_missing_one_is_silence() {
        let mut r = recorder();
        r.mix(1_000_000, &[1000; FRAME_PCM], 1_000_000);
        r.mix(1_005_000, &[500; FRAME_PCM], 1_000_000);
        r.mix(1_040_000, &[7; FRAME_PCM], 1_000_000);
        assert_eq!(r.steps[&0][0], 1500, "two pieces of one step add up");
        r.mix(500_000, &[9; FRAME_PCM], 1_000_000);
        r.mix(9_000_000, &[9; FRAME_PCM], 1_000_000);
        assert_eq!(r.steps.len(), 2, "before the start, and too far ahead, are dropped");
        // Nothing is written before the first video keyframe starts the file; the steps are taken all the same.
        r.sound_until(1_060_000).unwrap();
        assert_eq!(r.next_step, 3);
        assert!(r.steps.is_empty());
        r.mix(1_020_000, &[5; FRAME_PCM], 1_100_000);
        assert!(r.steps.is_empty(), "a step already written takes nothing late");
    }

    #[test]
    fn the_sound_ends_where_the_recording_does_with_its_last_samples() {
        let mut r = recorder();
        let mut file = r.video(&Arc::new(vec![0; 320 * 240]), &Sprite::default(), 0).unwrap();
        // A tone to the end, which falls 10 ms into a step.
        let tone = |i: usize| ((i as f64 * 2.0 * std::f64::consts::PI * 440.0 / 48_000.0).sin() * 8000.0) as i16;
        for step in 0..=50 {
            let pcm: Vec<i16> = (0..FRAME_SAMPLES).flat_map(|k| [tone(step * FRAME_SAMPLES + k); 2]).collect();
            let at = r.start_us + step as u64 * AUDIO_FRAME_US;
            r.mix(at, &pcm, at);
        }
        file.extend(r.sound_end(r.start_us + 1_010_000).unwrap());
        file.extend(r.webm.finish());
        let mut decoder = Decoder::new().unwrap();
        let (mut pcm, mut discard_ns) = (Vec::new(), 0);
        for (packet, discard) in super::super::webm::tests::sound(&file) {
            pcm.extend(decoder.decode(&packet).unwrap());
            discard_ns = discard;
        }
        // What a player plays: past the pre-skip, short of the last block's padding.
        let discard = ((discard_ns * 48 + 500_000) / 1_000_000) as usize;
        let played: Vec<i16> =
            pcm[2 * usize::from(r.pre_skip)..pcm.len() - 2 * discard].iter().step_by(2).copied().collect();
        assert_eq!(played.len(), 48_480, "exactly the recording's 1.01 s");
        // Its last 5 ms are the tone's, sample for sample (give or take the codec's loss).
        let last = (48_480 - 240..48_480).map(|i| (f64::from(played[i]), f64::from(tone(i))));
        let (dot, own, tone_energy) = last.fold((0.0, 0.0, 0.0), |(d, a, b), (p, t)| (d + p * t, a + p * p, b + t * t));
        let correlation = dot / (own * tone_energy).sqrt();
        assert!(correlation > 0.9, "the tone's end correlates {correlation:.3} with what plays");
    }

    /// A step takes every packet stamped for it, with no count bound: its sum stops at the ends of `i32` instead of
    /// overflowing (these tests run the debug build, which checks every addition), and is written clamped.
    #[test]
    fn a_step_flooded_with_full_scale_sound_saturates() {
        let mut r = recorder();
        r.steps.insert(0, vec![i32::MAX - 10; FRAME_PCM]);
        r.mix(1_000_000, &[i16::MAX; FRAME_PCM], 1_000_000);
        assert_eq!(r.steps[&0][0], i32::MAX);
        r.steps.insert(1, vec![i32::MIN + 10; FRAME_PCM]);
        r.mix(1_020_000, &[i16::MIN; FRAME_PCM], 1_000_000);
        assert_eq!(r.steps[&1][0], i32::MIN);
        r.sound_until(1_040_000).expect("the saturated steps are written");
    }

    #[test]
    fn frames_carry_the_camera_and_the_file_starts_with_a_keyframe() {
        let mut r = recorder();
        let screen = Arc::new(vec![0xff10_2030u32; 320 * 240]);
        // A small JPEG would need an encoder here; a decoded picture stands in for it.
        r.photo = Some(Photo { rgb: [0, 255, 0].repeat(64 * 48), width: 64, height: 48, at: Instant::now() });
        r.make_card();
        let none = Sprite::default();
        let first = r.video(&screen, &none, 0).unwrap();
        assert!(first.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]), "the WebM header comes with the first keyframe");
        let (x, y, w, h) = r.rect;
        assert_eq!(r.work[(y + h / 2) * 320 + x + w / 2], 0xff00_ff00, "the camera is drawn on the frame");
        assert!(r.video(&screen, &none, 1).unwrap().is_empty(), "a frame waits in its cluster");
        assert!(r.video(&Arc::new(vec![0; 4]), &none, 2).is_err(), "a screen of another size is refused");
        // Moved and restyled: the same picture, in its new place and shape.
        r.set_layout(CameraLayout { x: 0, y: 0, radius: 100, ..r.layout });
        r.video(&screen, &none, 3).unwrap();
        let (x, y, w, h) = r.rect;
        assert_eq!((x, y), (0, 0));
        assert_eq!(r.work[(y + h / 2) * 320 + x + w / 2], 0xff00_ff00, "drawn again from the last picture");
        assert_eq!(r.work[0], 0xff10_2030, "outside the circle, the screen");
    }

    #[test]
    fn the_card_shows_the_pictures_centre_and_leaves_the_screen_around_it() {
        let mut r = recorder();
        let screen = Arc::new(vec![0xff10_2030u32; 320 * 240]);
        let (red, green, blue) = (|p: u32| (p >> 16) & 0xff, |p: u32| (p >> 8) & 0xff, |p: u32| p & 0xff);
        // Each picture pixel says where it is: red its column, green its row. Wide, tall, and of odd sides.
        for (width, height) in [(96, 48), (48, 96), (37, 91)] {
            let rgb = (0..width * height).flat_map(|i| [(i % width) as u8, (i / width) as u8, 0]).collect();
            r.photo = Some(Photo { rgb, width, height, at: Instant::now() });
            let side = width.min(height) as u32;
            let (first_x, first_y) = ((width as u32 - side) / 2, (height as u32 - side) / 2);
            r.set_layout(CameraLayout { radius: 0, ..r.layout });
            r.work.copy_from_slice(&screen);
            r.draw_card();
            let (x0, y0, w, h) = r.rect;
            let at = |x: usize, y: usize| r.work[(y0 + y) * 320 + x0 + x];
            let case = format!("{width}x{height}");
            for (x, y) in [(0, h / 2), (w - 1, h / 2), (w / 2, 0), (w / 2, h - 1)] {
                assert_eq!(blue(at(x, y)), 0, "{case}: the picture to the edge at ({x}, {y})");
            }
            // The picture's centred square, scaled to cover the card: in order, end to end.
            let row: Vec<u32> = (0..w).map(|x| red(at(x, h / 2))).collect();
            let column: Vec<u32> = (0..h).map(|y| green(at(w / 2, y))).collect();
            for (along, first, what) in [(&row, first_x, "columns"), (&column, first_y, "rows")] {
                assert!(along.windows(2).all(|p| p[0] <= p[1]), "{case}: the {what} in order");
                // Give or take a pixel of scaling.
                let (start, end) = (along[0], *along.last().unwrap());
                assert!(start >= first && start <= first + 1, "{case}: the {what} start at the square's");
                assert!(end < first + side && end + 2 >= first + side, "{case}: the {what} end at the square's");
            }
            for (i, &pixel) in r.work.iter().enumerate() {
                let (x, y) = (i % 320, i / 320);
                if !(x0..x0 + w).contains(&x) || !(y0..y0 + h).contains(&y) {
                    assert_eq!(pixel, 0xff10_2030, "{case}: the screen around the card at ({x}, {y})");
                }
            }
        }
        // A circle: the screen at its corner, the picture at its middle, and the two blended along its edge.
        r.set_layout(CameraLayout { radius: 100, ..r.layout });
        r.work.copy_from_slice(&screen);
        r.draw_card();
        let (x0, y0, w, h) = r.rect;
        let at = |x: usize, y: usize| r.work[(y0 + y) * 320 + x0 + x];
        assert_eq!(at(0, 0), 0xff10_2030);
        assert_eq!(blue(at(w / 2, h / 2)), 0);
        assert!((0..w / 2).any(|k| (1..0x30).contains(&blue(at(k, k)))), "a smooth edge");
    }

    #[test]
    fn keyframes_follow_the_last_one_whatever_frames_are_skipped() {
        let mut r = recorder();
        let screen = Arc::new(vec![0u32; 320 * 240]);
        let none = Sprite::default();
        // The first frame made is a keyframe even when it is not frame 0.
        r.video(&screen, &none, 3).unwrap();
        assert_eq!(r.keyframe_at, Some(3));
        r.video(&screen, &none, 62).unwrap();
        assert_eq!(r.keyframe_at, Some(3), "not yet two seconds after it");
        r.video(&screen, &none, 64).unwrap();
        assert_eq!(r.keyframe_at, Some(64), "frame 63 was skipped: the next frame is the keyframe");
    }

    #[test]
    fn a_still_frame_is_encoded_until_sharp_then_left_out_until_it_changes() {
        let mut r = recorder();
        let screen = Arc::new(vec![0u32; 4]);
        let shown = |screen: &Arc<Vec<u32>>, at: Option<(u16, u16)>, card: Option<u64>| Shown {
            screen: screen.clone(),
            pointer: at.map(|at| (at, ARROW.clone())),
            card,
        };
        for k in 0..=STILL_FRAMES {
            assert!(!r.still(shown(&screen, None, None)), "frame {k} sharpens the still picture");
        }
        assert!(r.still(shown(&screen, None, None)), "then the same picture is left out");
        // Equal pixels in another buffer are a new screen: the scene paints a new one only where something changed.
        assert!(!r.still(shown(&Arc::new(vec![0u32; 4]), None, None)), "a new screen");
        assert!(!r.still(shown(&screen, None, None)));
        assert!(!r.still(shown(&screen, Some((10, 10)), None)), "the pointer shown");
        assert!(!r.still(shown(&screen, Some((11, 10)), None)), "the pointer moved");
        assert!(!r.still(shown(&screen, Some((11, 10)), Some(1))), "the camera's card shown");
        assert!(!r.still(shown(&screen, Some((11, 10)), Some(2))), "a new picture on the card");
    }

    #[test]
    fn a_keyframe_due_is_encoded_whatever_it_shows() {
        let mut r = recorder();
        let screen = Arc::new(vec![0xff10_2030u32; 320 * 240]);
        let none = Sprite::default();
        for frame in 0..KEYFRAME_EVERY {
            r.video(&screen, &none, frame).unwrap();
        }
        assert_eq!(r.keyframe_at, Some(0));
        r.video(&screen, &none, KEYFRAME_EVERY).unwrap();
        assert_eq!(r.keyframe_at, Some(KEYFRAME_EVERY), "a still screen still gets its keyframe");
    }

    #[test]
    fn the_pointer_is_drawn_at_its_point_and_cut_at_the_edges() {
        let mut r = recorder();
        let screen = Arc::new(vec![0xff10_2030u32; 320 * 240]);
        let arrow = Sprite { at: (100, 50), picture: Some(ARROW.clone()) };
        r.video(&screen, &arrow, 0).unwrap();
        assert_eq!(r.work[50 * 320 + 100], 0xff00_0000, "the arrow's tip is on the point");
        assert_eq!(r.work[53 * 320 + 101], 0xffff_ffff, "white inside");
        assert_eq!(r.work[50 * 320 + 101], 0xff10_2030, "the screen around it is untouched");
        // At the bottom-right corner, only what fits is drawn.
        let corner = Sprite { at: (319, 239), picture: Some(ARROW.clone()) };
        r.video(&screen, &corner, 1).unwrap();
        assert_eq!(r.work[239 * 320 + 319], 0xff00_0000);
    }

    #[test]
    fn hidden_and_named_pointers_have_their_pictures() {
        use super::super::cursor::Pointer;
        assert!(picture(&Pointer { shape: 0, ..Pointer::default() }).is_none(), "hidden");
        let text = picture(&Pointer { shape: 9, ..Pointer::default() }).unwrap();
        assert_eq!((text.width, text.height), (12, 19), "a named shape is the arrow");
        let rgba = [0u8, 0, 255, 128].repeat(4);
        let image = qoi::encode_to_vec(rgba, 2, 2).unwrap();
        let custom = Pointer {
            shape: super::super::cursor::IMAGE,
            hot_x: 1,
            hot_y: 1,
            image: Arc::from(image),
            ..Pointer::default()
        };
        let custom = picture(&custom).unwrap();
        assert_eq!((custom.width, custom.hot), (2, (1, 1)));
        assert_eq!(custom.pixels[0], 0x8000_0080, "straight alpha becomes premultiplied");
    }
}
