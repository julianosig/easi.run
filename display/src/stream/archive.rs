//! A recorder's stream (`Link::Archive`), in place of a viewer's encoder: the whole screen as one AV1 video, smaller than
//! the screen and at a constant quality, for a replay that shows what happened and costs little to keep. A frame goes out
//! only when the screen changed, at most [`FPS`] a second, so a still screen costs nothing; one that is behind skips
//! frames, as a viewer's does.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use display_protocol::frame::{Archive, svt_av1_crashes};
use display_protocol::{Codec, Message, Rect as WireRect, VERSION};

use super::av1::Av1;
use super::encoder::cursor_message;
use super::viewer::{Mailbox, Outbox, QUEUE_BUDGET, send, send_media_locked};

/// Frames a second at most: enough to follow what happened.
pub const FPS: u8 = 5;
/// Below the compositor and the live encoders, as a recording's worker.
const NICENESS: i32 = 10;

/// `side` at `percent`, even (the picture's color planes are half its size).
fn scaled(side: i32, percent: u8) -> u32 {
    (((side.max(2) as u32 * u32::from(percent)) / 100) & !1).max(2)
}

/// A screen's size at `percent`, and never one the encoder crashes on: a screen that passes can shrink to one that does.
fn shrunk(width: i32, height: i32, percent: u8) -> (u32, u32) {
    let (to_w, mut to_h) = (scaled(width, percent), scaled(height, percent));
    while svt_av1_crashes(to_w, to_h) {
        to_h -= 2;
    }
    (to_w, to_h)
}

/// A picture made smaller, each new pixel weighing the two nearest columns and the two nearest rows of the old one (at
/// half the size, the plain mean of four).
pub struct Shrink {
    from: (usize, usize),
    to: (usize, usize),
    /// For each new column: the old column on its left, and how much of the next one it takes, of 256.
    columns: Vec<(u32, u32)>,
    rows: Vec<(u32, u32)>,
    out: Vec<u32>,
}

/// Where each of `to` new places falls among `from` old ones, centre on centre.
fn places(from: usize, to: usize) -> Vec<(u32, u32)> {
    (0..to as u64)
        .map(|i| {
            let at = ((2 * i + 1) * from as u64 * 128 / to as u64).saturating_sub(128);
            let left = ((at >> 8) as usize).min(from.saturating_sub(2)) as u32;
            (left, (at - (u64::from(left) << 8)).min(256) as u32)
        })
        .collect()
}

impl Shrink {
    pub fn new(from: (usize, usize), to: (usize, usize)) -> Shrink {
        Shrink { from, to, columns: places(from.0, to.0), rows: places(from.1, to.1), out: vec![0; to.0 * to.1] }
    }

    pub fn apply(&mut self, pixels: &[u32]) -> &[u32] {
        let (from_w, _) = self.from;
        let (to_w, _) = self.to;
        if self.from == self.to {
            self.out.copy_from_slice(pixels);
            return &self.out;
        }
        for (row, out) in self.rows.iter().zip(self.out.chunks_exact_mut(to_w)) {
            let (y, wy) = (row.0 as usize, row.1);
            let (top, bottom) = (&pixels[y * from_w..(y + 1) * from_w], &pixels[(y + 1) * from_w..(y + 2) * from_w]);
            for (&(x, wx), pixel) in self.columns.iter().zip(out.iter_mut()) {
                let x = x as usize;
                let (a, b, c, d) = (top[x], top[x + 1], bottom[x], bottom[x + 1]);
                let (ia, ib, ic, id) = ((256 - wx) * (256 - wy), wx * (256 - wy), (256 - wx) * wy, wx * wy);
                let channel = |shift: u32| -> u32 {
                    let of = |p: u32| (p >> shift) & 0xff;
                    ((of(a) * ia + of(b) * ib + of(c) * ic + of(d) * id + (1 << 15)) >> 16) << shift
                };
                *pixel = channel(0) | channel(8) | channel(16);
            }
        }
        &self.out
    }
}

/// Runs the recorder's stream until its connection closes.
pub(super) fn run(mailbox: Arc<Mailbox>, out: Outbox, archive: Archive) {
    let _ = rustix::process::setpriority_process(None, NICENESS);
    let (width, height) = (mailbox.width, mailbox.height);
    let to = shrunk(width, height, archive.scale);
    let mut encoder = match Av1::archive(to.0, to.1, archive.crf, FPS) {
        Ok(encoder) => encoder,
        Err(e) => {
            tracing::error!(%e, "the recorder's encoder failed to start");
            mailbox.close();
            return;
        }
    };
    let mut shrink = Shrink::new((width as usize, height as usize), (to.0 as usize, to.1 as usize));
    let config = Message::Config {
        version: VERSION,
        epoch: 1,
        width: width as u16,
        height: height as u16,
        codec: Codec::Av1,
        max_fps: FPS,
    };
    mailbox.epoch.store(1, Ordering::Relaxed);
    if !send(&out, &mailbox, &config) {
        return;
    }
    mailbox.configured.store(true, Ordering::Relaxed);
    let every = Duration::from_secs(1) / u32::from(FPS);
    let whole = WireRect { x: 0, y: 0, w: width as u16, h: height as u16 };
    let (mut revision, mut keyframe, mut next) = (0u64, true, Instant::now());
    loop {
        let (snapshot, wanted) = {
            let mut pending = mailbox.pending.lock().unwrap();
            pending.pointer_dirty |= revision == 0 && pending.pointer.is_some();
            loop {
                if pending.closed {
                    return;
                }
                let room = mailbox.queued.load(Ordering::Relaxed) < QUEUE_BUDGET;
                if pending.pointer_dirty && room {
                    pending.pointer_dirty = false;
                    revision += 1;
                    let pointer = pending.pointer.as_ref().expect("a dirty pointer exists");
                    let message = cursor_message(1, revision, pointer);
                    if !send_media_locked(&out, &mailbox, &mut pending, &message) {
                        return;
                    }
                }
                let wanted = keyframe || pending.refresh;
                let now = Instant::now();
                if room && now >= next && pending.latest.is_some() && (wanted || !pending.dirty.is_empty()) {
                    pending.refresh = false;
                    pending.dirty.take();
                    pending.unobserved.take();
                    pending.compositions.clear();
                    break (pending.latest.clone().expect("checked above"), wanted);
                }
                // Past its turn it waits to be woken (a change, a request), or for room to free up; before it, for its turn.
                let wait = if now < next { next - now } else { every };
                pending = mailbox.wake.wait_timeout(pending, wait).unwrap().0;
            }
        };
        let started = Instant::now();
        next = started + every;
        let picture = shrink.apply(&snapshot.pixels);
        match encoder.encode(picture, to.0 as usize, wanted) {
            Ok(payload) if !payload.is_empty() => {
                keyframe &= !wanted;
                revision += 1;
                let message = Message::Video {
                    epoch: 1,
                    revision,
                    rect: whole,
                    keyframe: wanted,
                    timestamp_us: snapshot.time_us,
                    payload,
                };
                let mut pending = mailbox.pending.lock().unwrap();
                if !send_media_locked(&out, &mailbox, &mut pending, &message) {
                    return;
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!(%e, "the recorder's encoding failed");
                mailbox.close();
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_shrunk_keeps_its_colors_and_means_what_it_covers() {
        let flat = vec![0x00_40_80_c0u32; 1280 * 800];
        let mut three_quarters = Shrink::new((1280, 800), (960, 600));
        assert!(three_quarters.apply(&flat).iter().all(|&p| p == 0x00_40_80_c0));
        // Half the size: each new pixel is the mean of the four it covers.
        let picture: Vec<u32> = (0..16).map(|i| i * 16).collect();
        let mut half = Shrink::new((4, 4), (2, 2));
        assert_eq!(half.apply(&picture), [40, 72, 168, 200]);
        // The same size, unchanged.
        let mut same = Shrink::new((4, 4), (4, 4));
        assert_eq!(same.apply(&picture), &picture[..]);
    }

    #[test]
    fn sizes_stay_even_for_the_picture_s_color_planes() {
        assert_eq!((scaled(1280, 75), scaled(800, 75)), (960, 600));
        assert_eq!((scaled(1367, 50), scaled(769, 100)), (682, 768));
    }

    #[test]
    fn a_screen_never_shrinks_to_a_size_the_encoder_crashes_on() {
        assert!(svt_av1_crashes(962, 654), "75% of an accepted 1284x872");
        assert_eq!(shrunk(1284, 872, 75), (962, 648));
        assert_eq!(shrunk(1280, 800, 75), (960, 600));
        for width in (320..=1920).step_by(2) {
            for height in (240..=1080).step_by(2) {
                for percent in [50, 75, 100] {
                    let (w, h) = shrunk(width, height, percent);
                    assert!(!svt_av1_crashes(w, h), "{width}x{height} at {percent}%");
                }
            }
        }
    }
}
