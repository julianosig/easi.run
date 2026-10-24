//! Motion detection from damage: a short window in which many frames changed a large share of the output (scrolling,
//! video, animation) switches the stream to video; a quiet spell switches it back to lossless tiles.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The window is long enough to hold several frames of slow sources: a browser short of CPU plays a video at 10-15
/// fps, and video at that rate must still count as motion rather than go out as lossless tiles every frame.
const WINDOW: Duration = Duration::from_millis(500);
/// At least 8 fps over the window.
const MIN_FRAMES: usize = 4;
/// Summed damaged area over the window, as a share of the output: 2.4 outputs a second.
const MIN_AREA_SHARE: f64 = 1.2;
/// How long the stream stays in video after the last frame that would have started it.
const HOLD: Duration = Duration::from_millis(400);

#[derive(Default)]
pub struct Motion {
    frames: VecDeque<(Instant, u64)>,
    last_motion: Option<Instant>,
}

impl Motion {
    /// Records one composed frame's damaged area.
    pub fn update(&mut self, now: Instant, damaged: u64, output: u64) {
        // An episode that ended starts the next one from nothing: its frames say nothing about this change.
        if self.last_motion.is_some() && !self.active(now) {
            self.last_motion = None;
            self.frames.clear();
        }
        self.frames.push_back((now, damaged));
        while self.frames.front().is_some_and(|(t, _)| now.duration_since(*t) > WINDOW) {
            self.frames.pop_front();
        }
        let area: u64 = self.frames.iter().map(|(_, a)| a).sum();
        if self.frames.len() >= MIN_FRAMES && area as f64 >= MIN_AREA_SHARE * output as f64 {
            self.last_motion = Some(now);
        }
    }

    pub fn active(&self, now: Instant) -> bool {
        self.last_motion.is_some_and(|t| now.duration_since(t) <= HOLD)
    }

    /// When the stream should next look again because the hold expires.
    pub fn hold_ends(&self) -> Option<Instant> {
        self.last_motion.map(|t| t + HOLD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_frequent_damage_is_motion_and_small_damage_is_not() {
        let start = Instant::now();
        let output = 1280 * 800;
        let mut typing = Motion::default();
        for i in 0..30 {
            let now = start + Duration::from_millis(i * 16);
            typing.update(now, 20 * 20, output);
            assert!(!typing.active(now));
        }
        let mut scroll = Motion::default();
        for i in 0..6 {
            scroll.update(start + Duration::from_millis(i * 16), output / 2, output);
        }
        assert!(scroll.active(start + Duration::from_millis(5 * 16)));
        assert!(scroll.active(start + Duration::from_millis(5 * 16 + 300)));
        assert!(!scroll.active(start + Duration::from_millis(5 * 16 + 500)));
    }

    #[test]
    fn a_video_at_a_starved_browsers_frame_rate_is_motion() {
        let start = Instant::now();
        let output = 1280 * 800;
        // A video over 70% of the output at 13 fps, as a browser short of CPU plays it.
        let mut video = Motion::default();
        for i in 0..8 {
            video.update(start + Duration::from_millis(i * 77), output * 7 / 10, output);
        }
        assert!(video.active(start + Duration::from_millis(7 * 77)));
        // One still update after an episode ended does not start another.
        let mut after = Motion::default();
        for i in 0..6 {
            after.update(start + Duration::from_millis(i * 16), output, output);
        }
        let later = start + Duration::from_millis(5 * 16 + 460);
        after.update(later, output, output);
        assert!(!after.active(later));
        // A small spinner at 13 fps is not.
        let mut spinner = Motion::default();
        assert!((0..13).all(|i| {
            let now = start + Duration::from_millis(i * 77);
            spinner.update(now, output / 20, output);
            !spinner.active(now)
        }));
    }
}
