//! The browser's sound: 48 kHz stereo 16-bit samples that the browser container's sound daemon writes to a FIFO the
//! display owns, encoded once as Opus and sent to every viewer.
//!
//! The FIFO carries samples and nothing else, so no byte of it is parsed: any content is valid sound. The daemon paces
//! itself by the clock; the display reads at most real time, and a backlog over [`MAX_BACKLOG`] (which only a writer
//! that grows the FIFO past its [`PIPE_SIZE`] can build) is discarded unheard, a bounded amount per turn, so a writer
//! flooding the FIFO costs the display bounded work and is held to real time. The
//! display opens it read-write, so the FIFO never reports end of file and the thread sleeps in `poll` while nothing
//! plays.
//!
//! Discontinuities: discarded sound, a pause of [`STALL`] in the samples, or a stream whose timestamps fall behind the
//! clock by [`MAX_LAG`] drop the unfinished packet and start a new stream at the clock (the sound was already
//! interrupted, so the encoder's tail is not played out). Silence ends a stream after [`TAIL_FRAMES`] silent packets,
//! which play it out. The encoder goes on across a new stream, as a viewer's decoder does, so a viewer that received
//! every packet stays in step with it: a new stream is a gap in the timestamps, never a reset.

use std::os::fd::{AsFd, OwnedFd};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use display_protocol::{AUDIO_FRAME_US, Message};
use rustix::event::{PollFd, PollFlags, Timespec};
use rustix::fs::{FileType, Mode, OFlags};

use super::opus::{FRAME_BYTES, FRAME_SAMPLES, Opus};
use super::viewer::Viewers;

/// Bytes of sound per millisecond.
const BYTES_PER_MS: usize = FRAME_BYTES / (AUDIO_FRAME_US as usize / 1000);
/// The most sound one turn may take in, so the reader never runs ahead of real time by more than this.
const MAX_TOKENS: usize = 100 * BYTES_PER_MS;
/// Sound waiting in the FIFO beyond this is old: it is discarded and the stream restarts.
const MAX_BACKLOG: usize = 100 * BYTES_PER_MS;
/// The most a turn discards.
const MAX_DISCARD: usize = 64 * 1024;
/// The shortest time between two turns.
const TURN: Duration = Duration::from_millis(10);
/// Silent packets sent after the sound stops, so the decoder plays out the encoder's lookahead.
const TAIL_FRAMES: u32 = 5;
/// A stream with no samples for this long ends.
const STALL: Duration = Duration::from_millis(60);
/// A stream whose next timestamp is this far behind the clock restarts at the clock.
const MAX_LAG: u64 = 200_000;
/// The kernel's buffer: short, since the daemon fills it as fast as the clock allows (a latency, not a limit; the
/// writer may resize it).
const PIPE_SIZE: usize = 8192;
/// Creates the FIFO at `path` (replacing whatever the display's directory held there), shared with the directory's
/// group, and opens it.
pub fn create(path: &Path) -> Result<OwnedFd, String> {
    let at = |e: rustix::io::Errno| format!("{}: {e}", path.display());
    match rustix::fs::unlink(path) {
        Ok(()) | Err(rustix::io::Errno::NOENT) => {}
        Err(e) => return Err(at(e)),
    }
    rustix::fs::mknodat(rustix::fs::CWD, path, FileType::Fifo, Mode::from_raw_mode(0o600), 0).map_err(at)?;
    crate::share_with_directory_group(path)?;
    let fd =
        rustix::fs::open(path, OFlags::RDWR | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOFOLLOW, Mode::empty())
            .map_err(at)?;
    let stat = rustix::fs::fstat(&fd).map_err(at)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Fifo || stat.st_uid != rustix::process::geteuid().as_raw() {
        return Err(format!("{} is not the display's own FIFO", path.display()));
    }
    let _ = rustix::pipe::fcntl_setpipe_size(&fd, PIPE_SIZE);
    Ok(fd)
}

/// Reads sound from `fifo` and sends it to `listeners` until the process ends, and its samples to a recording while
/// one runs (`tap`).
pub fn start(fifo: OwnedFd, viewers: Arc<Viewers>, tap: crate::recording::Tap) {
    let spawned = std::thread::Builder::new().name("audio".into()).spawn(move || {
        if let Err(e) = run(fifo, &viewers, &tap) {
            tracing::error!(%e, "the browser's sound stopped");
        }
    });
    if let Err(e) = spawned {
        tracing::error!(%e, "the browser's sound did not start");
    }
}

fn run(fifo: OwnedFd, viewers: &Viewers, tap: &crate::recording::Tap) -> Result<(), String> {
    let mut opus = Opus::new()?;
    let mut framer = Framer::default();
    let mut buffer = vec![0u8; MAX_TOKENS.max(MAX_DISCARD)];
    let mut last_turn = Instant::now();
    loop {
        // Asleep while nothing plays; while samples flow, awake at least often enough to notice they stopped.
        let timeout = framer.active().then(|| Timespec::try_from(STALL).expect("a short timeout"));
        let mut fds = [PollFd::new(&fifo, PollFlags::IN)];
        match rustix::event::poll(&mut fds, timeout.as_ref()) {
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(e) => return Err(format!("poll: {e}")),
        }
        let since = last_turn.elapsed();
        if since < TURN {
            std::thread::sleep(TURN - since);
        }
        let now = Instant::now();
        let elapsed = now - last_turn;
        last_turn = now;
        let available = rustix::io::ioctl_fionread(&fifo).map_err(|e| format!("FIONREAD: {e}"))? as usize;
        let take = framer.admit(elapsed, available);
        if take.discard > 0 {
            read_into(&fifo, &mut buffer[..take.discard])?;
        }
        let n = read_into(&fifo, &mut buffer[..take.read])?;
        let frames = framer.push(&buffer[..n], super::clock_us(), Instant::now());
        if frames.is_empty() || !viewers.any() {
            continue;
        }
        // A recording takes the samples themselves (it mixes and encodes them its own way); a full queue drops them.
        if let Some(recording) = tap.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
            for frame in &frames {
                let _ = recording.try_send(super::record::Input::Sound {
                    timestamp_us: frame.timestamp_us,
                    pcm: frame.pcm.to_vec(),
                });
            }
        }
        for frame in frames {
            let packet = opus.encode(&frame.pcm)?;
            viewers.broadcast_sound(&Message::Audio { timestamp_us: frame.timestamp_us, payload: packet });
        }
    }
}

/// Reads up to `buffer.len()` bytes without blocking.
fn read_into(fifo: &impl AsFd, buffer: &mut [u8]) -> Result<usize, String> {
    let mut filled = 0;
    while filled < buffer.len() {
        match rustix::io::read(fifo, &mut buffer[filled..]) {
            Ok(0) | Err(rustix::io::Errno::AGAIN) => break,
            Ok(n) => filled += n,
            Err(rustix::io::Errno::INTR) => {}
            Err(e) => return Err(format!("read: {e}")),
        }
    }
    Ok(filled)
}

/// What one turn takes from the FIFO.
#[derive(Debug, PartialEq, Eq)]
struct Take {
    discard: usize,
    read: usize,
}

/// One packet's samples and when they start.
struct Frame {
    timestamp_us: u64,
    pcm: [i16; FRAME_SAMPLES * 2],
}

/// Turns the FIFO's bytes into packets: at most real time, whole stereo samples, no packets while silent, and a new
/// stream after silence or discarded sound.
#[derive(Default)]
struct Framer {
    /// Real-time allowance, in bytes, capped at [`MAX_TOKENS`].
    tokens: usize,
    /// Bytes of the packet being filled.
    partial: Vec<u8>,
    /// The next packet's timestamp, while a stream runs.
    next_us: Option<u64>,
    /// Silent packets sent since the last sound.
    silent: u32,
    /// When sound last arrived.
    last_sound: Option<Instant>,
    /// A backlog is being discarded; it stays so until the FIFO holds no more than real time added since the last turn.
    rejecting: bool,
}

impl Framer {
    /// A stream runs or a packet is being filled.
    fn active(&self) -> bool {
        self.next_us.is_some() || !self.partial.is_empty()
    }

    /// How much of `available` bytes to discard and to read after `elapsed`.
    fn admit(&mut self, elapsed: Duration, available: usize) -> Take {
        let earned = (elapsed.as_micros() as usize).saturating_mul(BYTES_PER_MS) / 1000;
        self.tokens = self.tokens.saturating_add(earned).min(MAX_TOKENS);
        // What real time adds in one turn, at most a turn's allowance (so the drop below stays within MAX_DISCARD).
        if self.rejecting && available <= earned.min(MAX_TOKENS) {
            // Drained to what real time adds in a turn: that too is dropped, and the next turn reads again.
            self.rejecting = false;
            return Take { discard: available / 4 * 4, read: 0 };
        }
        if self.rejecting || available > self.tokens + MAX_BACKLOG {
            // Old sound, or a writer running ahead of real time: dropped unheard, in whole samples, all of it, and the
            // stream starts over once the backlog is gone.
            self.rejecting = true;
            self.end();
            return Take { discard: available.min(MAX_DISCARD) / 4 * 4, read: 0 };
        }
        // Whole stereo samples only, so every byte consumed keeps the samples aligned.
        let read = available.min(self.tokens) / 4 * 4;
        self.tokens -= read;
        Take { discard: 0, read }
    }

    fn end(&mut self) {
        self.partial.clear();
        self.next_us = None;
        self.silent = 0;
    }

    /// Takes `bytes` read at `now_us` (the display clock) and `now`, and returns the packets they complete.
    fn push(&mut self, bytes: &[u8], now_us: u64, now: Instant) -> Vec<Frame> {
        let paused = self.last_sound.is_some_and(|t| now - t >= STALL);
        let behind = self.next_us.is_some_and(|next| now_us > next + MAX_LAG);
        if paused || behind {
            self.end();
        }
        if bytes.is_empty() {
            return Vec::new();
        }
        self.last_sound = Some(now);
        let mut frames = Vec::new();
        let mut rest = bytes;
        while !rest.is_empty() {
            let n = (FRAME_BYTES - self.partial.len()).min(rest.len());
            self.partial.extend_from_slice(&rest[..n]);
            rest = &rest[n..];
            if self.partial.len() < FRAME_BYTES {
                break;
            }
            let silent = self.partial.iter().all(|&b| b == 0);
            let mut pcm = [0i16; FRAME_SAMPLES * 2];
            for (sample, pair) in pcm.iter_mut().zip(self.partial.as_chunks::<2>().0) {
                *sample = i16::from_le_bytes(*pair);
            }
            self.partial.clear();
            if silent {
                if self.next_us.is_none() {
                    continue;
                }
                self.silent += 1;
                if self.silent > TAIL_FRAMES {
                    self.end();
                    continue;
                }
            } else {
                self.silent = 0;
            }
            // A new stream starts when its first sample was read: the bytes after this packet's end came later.
            let after = (rest.len() / BYTES_PER_MS) as u64 * 1000;
            let timestamp_us = self.next_us.unwrap_or_else(|| now_us.saturating_sub(after + AUDIO_FRAME_US));
            self.next_us = Some(timestamp_us + AUDIO_FRAME_US);
            frames.push(Frame { timestamp_us, pcm });
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frames: usize) -> Vec<u8> {
        (0..frames * FRAME_SAMPLES * 2).flat_map(|i| ((i % 64) as i16 * 100 + 1).to_le_bytes()).collect()
    }

    #[test]
    fn whole_packets_come_from_any_split_of_the_bytes() {
        let mut framer = Framer::default();
        let bytes = tone(3);
        let mut frames = Vec::new();
        let now = Instant::now();
        for chunk in bytes.chunks(1001) {
            frames.extend(framer.push(chunk, 1_000_000, now));
        }
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[1].timestamp_us, frames[0].timestamp_us + AUDIO_FRAME_US);
        assert_eq!(frames[2].timestamp_us, frames[1].timestamp_us + AUDIO_FRAME_US);
        let expected: Vec<i16> = bytes.chunks(2).map(|p| i16::from_le_bytes([p[0], p[1]])).collect();
        let got: Vec<i16> = frames.iter().flat_map(|f| f.pcm).collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn silence_sends_a_short_tail_then_nothing_and_sound_starts_a_new_stream() {
        let mut framer = Framer::default();
        let now = Instant::now();
        assert!(framer.push(&vec![0; FRAME_BYTES * 3], 0, now).is_empty(), "silence before any sound sends nothing");
        assert_eq!(framer.push(&tone(1), 100_000, now).len(), 1);
        let tail = framer.push(&vec![0; FRAME_BYTES * 20], 120_000, now);
        assert_eq!(tail.len(), TAIL_FRAMES as usize);
        assert!(!framer.active());
        let again = framer.push(&tone(1), 900_000, now);
        assert_eq!(again[0].timestamp_us, 900_000 - AUDIO_FRAME_US);
    }

    #[test]
    fn a_pause_or_a_lagging_clock_drops_the_unfinished_packet_and_restarts_the_stream() {
        let mut framer = Framer::default();
        let start = Instant::now();
        assert_eq!(framer.push(&tone(1), 1_000_000, start).len(), 1);
        // Half a packet, then nothing for longer than STALL: the half is dropped, not spliced into later sound.
        assert!(framer.push(&tone(1)[..FRAME_BYTES / 2], 1_020_000, start).is_empty());
        assert!(framer.active());
        assert!(framer.push(&[], 1_100_000, start + STALL).is_empty());
        assert!(!framer.active());
        let after_pause = framer.push(&tone(1), 1_200_000, start + STALL * 2);
        assert_eq!(after_pause.len(), 1);
        assert_eq!(after_pause[0].timestamp_us, 1_200_000 - AUDIO_FRAME_US);
        // Samples that keep coming while the stream's timestamps fall behind the clock: restarted at the clock.
        let now = start + STALL * 2;
        let late = framer.push(&tone(1), 1_200_000 + MAX_LAG + 100_000, now);
        assert_eq!(late[0].timestamp_us, 1_200_000 + MAX_LAG + 100_000 - AUDIO_FRAME_US);
    }

    #[test]
    fn reading_never_runs_ahead_of_real_time_and_a_backlog_is_discarded_unheard_in_bounded_steps() {
        let mut framer = Framer::default();
        // A flooded FIFO: ten seconds of sound waiting after 10 ms. Nothing of it is read to be heard.
        let take = framer.admit(Duration::from_millis(10), 10 * 1000 * BYTES_PER_MS);
        assert_eq!(take, Take { discard: MAX_DISCARD, read: 0 });
        assert_eq!(framer.admit(Duration::from_millis(20), MAX_TOKENS * 4), Take { discard: MAX_DISCARD, read: 0 });
        // The rest of the backlog is dropped too, down to its last bytes, however its size falls.
        let mut left = 1_021_040;
        while left > 0 {
            let take = framer.admit(Duration::from_millis(10), left);
            assert_eq!(take.read, 0, "{left} bytes of a rejected backlog were read to be heard");
            left -= take.discard;
        }
        // A long pause in the middle of a rejection still drops at most MAX_DISCARD a turn.
        framer.rejecting = true;
        assert_eq!(framer.admit(Duration::from_secs(1), 100_000), Take { discard: MAX_DISCARD, read: 0 });
        // An emptied FIFO ends the rejection.
        assert_eq!(framer.admit(Duration::from_millis(10), 0), Take { discard: 0, read: 0 });
        // A long sleep earns at most MAX_TOKENS.
        let take = framer.admit(Duration::from_secs(5), MAX_TOKENS);
        assert_eq!(take, Take { discard: 0, read: MAX_TOKENS });
        // A real-time writer is read entirely, with nothing discarded.
        for _ in 0..100 {
            assert_eq!(framer.admit(Duration::from_millis(10), 10 * BYTES_PER_MS), Take { discard: 0, read: 1920 });
        }
    }
}
