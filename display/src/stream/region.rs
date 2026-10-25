//! Where video goes during motion: the bounding box of the tiles whose pixels actually changed lately (damage is only
//! a hint, so each damaged tile's pixels are hashed and compared with its last hash), in whole tiles. The rest of the output stays lossless tiles, so while a video plays in one part of the
//! screen, a change anywhere else (a hover, a new page around the player) arrives exact rather than as video that
//! leaves faint traces of the previous picture.

use std::hash::{BuildHasher, RandomState};
use std::time::{Duration, Instant};

use super::tiles::{TILE, tile_rect, tiles_of};
use crate::compose::Rect;

/// Tiles that changed within this window count as changing.
pub const ACTIVE_WINDOW: Duration = Duration::from_millis(500);
/// Changes of a tile at most this far apart are one continuous motion.
pub const CONTINUOUS_GAP: Duration = Duration::from_millis(250);
/// A tile in continuous motion for this long is in sustained motion, as a video's are; a hover's transition, a
/// blinking caret, or a counter ticking each second is not.
pub const SUSTAINED: Duration = Duration::from_millis(500);
/// How long a tile that stopped after sustained motion must stay still before it counts as settled: a video's still
/// parts keep being refined like the rest of it, so they are repainted only after a longer rest, in case what stopped
/// was a long transition or an overlay that went right after it.
pub const SETTLED_AFTER_MOTION: Duration = Duration::from_secs(3);
/// Tiles that settled with their last changes this close together changed as one.
const SETTLED_TOGETHER: Duration = Duration::from_millis(250);
/// How long any changed tile, however few changed with it, must stay still before it counts as settled: small residues
/// are repainted too, only rarely.
pub const SETTLED_ALONE_AFTER: Duration = Duration::from_secs(10);

/// When each tile of the output last changed.
pub struct Activity {
    width: i32,
    height: i32,
    columns: i32,
    rows: i32,
    changed: Vec<Option<Instant>>,
    /// When each tile's current continuous motion began.
    since: Vec<Option<Instant>>,
    /// Each tile's pixels as last observed, hashed; a black output to start with.
    hashes: Vec<u64>,
    seed: u64,
}

impl Activity {
    pub fn new(width: i32, height: i32) -> Activity {
        let (columns, rows) = ((width + TILE - 1) / TILE, (height + TILE - 1) / TILE);
        // Random per display, so a page cannot work out pixels whose change goes unseen.
        let seed = RandomState::new().hash_one(0u8);
        let black = [0u32; TILE as usize];
        let mut hashes = Vec::with_capacity((columns * rows) as usize);
        for r in 0..rows {
            for c in 0..columns {
                let tile = tile_rect(c, r, width, height);
                hashes.push(hash(seed, std::iter::repeat_n(&black[..tile.w as usize], tile.h as usize)));
            }
        }
        let tiles = (columns * rows) as usize;
        Activity { width, height, columns, rows, changed: vec![None; tiles], since: vec![None; tiles], hashes, seed }
    }

    /// Records, at `now`, which tiles touched by `damage` hold other pixels than when last observed.
    pub fn observe(&mut self, pixels: &[u32], damage: &[Rect], now: Instant) {
        let mut seen = vec![false; self.hashes.len()];
        for part in damage.iter().filter(|p| !p.is_empty()) {
            for (c, r) in tiles_of(part, self.columns, self.rows) {
                let i = (r * self.columns + c) as usize;
                if std::mem::replace(&mut seen[i], true) {
                    continue;
                }
                let tile = tile_rect(c, r, self.width, self.height);
                let width = self.width as usize;
                let rows = (tile.y..tile.y + tile.h).map(|y| {
                    let at = y as usize * width + tile.x as usize;
                    &pixels[at..at + tile.w as usize]
                });
                let hash = hash(self.seed, rows);
                if std::mem::replace(&mut self.hashes[i], hash) != hash {
                    self.touch(i, now);
                }
            }
        }
    }

    /// Records that the pixels of `parts` changed at `now`.
    pub fn mark(&mut self, parts: &[Rect], now: Instant) {
        for part in parts.iter().filter(|p| !p.is_empty()) {
            for (c, r) in tiles_of(part, self.columns, self.rows) {
                self.touch((r * self.columns + c) as usize, now);
            }
        }
    }

    /// Records that tile `i` changed at `now`.
    fn touch(&mut self, i: usize, now: Instant) {
        if !self.changed[i].is_some_and(|t| now.saturating_duration_since(t) < CONTINUOUS_GAP) {
            self.since[i] = Some(now);
        }
        self.changed[i] = Some(now);
    }

    /// The region video covers at `now`: the bounding box of the tiles that changed within [`ACTIVE_WINDOW`], in whole
    /// tiles clipped to the output, and at least a whole tile on each side (a clipped edge tile takes its neighbour
    /// too, since a video encoder needs 64 pixels); `None` when nothing changed.
    pub fn region(&self, now: Instant) -> Option<Rect> {
        self.bounds(|i| self.changed[i].is_some_and(|t| now.saturating_duration_since(t) < ACTIVE_WINDOW))
    }

    /// Like [`Activity::region`], of the tiles in sustained motion only: changing within [`ACTIVE_WINDOW`], and
    /// continuously for [`SUSTAINED`].
    pub fn moving(&self, now: Instant) -> Option<Rect> {
        self.bounds(|i| match (self.changed[i], self.since[i]) {
            (Some(last), Some(since)) => {
                now.saturating_duration_since(last) < ACTIVE_WINDOW && last.duration_since(since) >= SUSTAINED
            }
            _ => false,
        })
    }

    /// The bounding box of the tiles `pick` takes, in whole tiles clipped to the output, at least a whole tile on each
    /// side.
    fn bounds(&self, pick: impl Fn(usize) -> bool) -> Option<Rect> {
        let (mut c0, mut r0, mut c1, mut r1) = (i32::MAX, i32::MAX, -1, -1);
        for i in 0..self.changed.len() {
            if pick(i) {
                let (c, r) = (i as i32 % self.columns, i as i32 / self.columns);
                (c0, r0, c1, r1) = (c0.min(c), r0.min(r), c1.max(c), r1.max(r));
            }
        }
        if c1 < 0 {
            return None;
        }
        let span = |first: i32, last: i32, size: i32| (first * TILE, ((last + 1) * TILE).min(size) - first * TILE);
        if span(c0, c1, self.width).1 < TILE && c0 > 0 {
            c0 -= 1;
        }
        if span(r0, r1, self.height).1 < TILE && r0 > 0 {
            r0 -= 1;
        }
        let ((x, w), (y, h)) = (span(c0, c1, self.width), span(r0, r1, self.height));
        Some(Rect::new(x, y, w, h))
    }

    /// Whether a part of `area` changed as one after `since` and has then been still for `quiet` (for
    /// [`SETTLED_AFTER_MOTION`] when it stopped after sustained motion) at `now`: the video's picture there may still
    /// hold traces of what it showed before, which only a keyframe clears. As one: a tenth of the area's tiles (at
    /// least four, or all of a smaller area) whose last changes came within [`SETTLED_TOGETHER`], as when a page or a
    /// panel changes; the few tiles a figure crossing a video leaves behind count only once still for
    /// [`SETTLED_ALONE_AFTER`].
    pub fn settled(&self, area: &Rect, since: Instant, quiet: Duration, now: Instant) -> bool {
        let (c0, r0) = (area.x / TILE, area.y / TILE);
        let (c1, r1) = ((area.x + area.w - 1) / TILE, (area.y + area.h - 1) / TILE);
        let (c1, r1) = (c1.min(self.columns - 1), r1.min(self.rows - 1));
        let tiles = ((c1 - c0 + 1) * (r1 - r0 + 1)).max(1) as usize;
        let needed = (tiles / 10).max(4).min(tiles);
        let mut lasts: Vec<Instant> = (r0..=r1)
            .flat_map(|r| (c0..=c1).map(move |c| (r * self.columns + c) as usize))
            .filter_map(|i| {
                let (last, began) = (self.changed[i]?, self.since[i]?);
                let quiet = if last.duration_since(began) >= SUSTAINED { SETTLED_AFTER_MOTION } else { quiet };
                (last > since && now.saturating_duration_since(last) >= quiet).then_some(last)
            })
            .collect();
        lasts.sort_unstable();
        lasts.first().is_some_and(|first| now.saturating_duration_since(*first) >= SETTLED_ALONE_AFTER)
            || lasts.windows(needed).any(|run| run[needed - 1].duration_since(run[0]) <= SETTLED_TOGETHER)
    }

    /// Whether no tile of `outer` outside `inner` changed within `window` before `now`.
    pub fn still_outside(&self, outer: &Rect, inner: &Rect, window: Duration, now: Instant) -> bool {
        tiles_outside(outer, Some(inner)).iter().all(|tile| {
            let i = ((tile.y / TILE) * self.columns + tile.x / TILE) as usize;
            self.changed[i].is_none_or(|t| now.saturating_duration_since(t) >= window)
        })
    }
}

/// A tile's pixels, row by row, hashed in four independent lanes so it runs at about the speed of reading them. Each
/// step of a lane is invertible in both its state and its input, so one changed pair of pixels always changes the
/// hash; changes that cancel out need many at once and depend on the random `seed`. A miss costs no pixel: the video
/// still codes the region, and changes outside it go as tiles compared exactly.
fn hash<'a>(seed: u64, rows: impl Iterator<Item = &'a [u32]>) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    let step = |lane: u64, word: u64| (lane ^ word).wrapping_mul(K).rotate_left(29);
    let pair = |p: &[u32; 2]| u64::from(p[0]) | u64::from(p[1]) << 32;
    let mut lanes = [seed, seed ^ 1, seed ^ 2, seed ^ 3];
    for row in rows {
        let (chunks, rest) = row.as_chunks::<8>();
        for chunk in chunks {
            for (lane, p) in lanes.iter_mut().zip(chunk.as_chunks::<2>().0) {
                *lane = step(*lane, pair(p));
            }
        }
        let (pairs, last) = rest.as_chunks::<2>();
        for p in pairs {
            lanes[0] = step(lanes[0], pair(p));
        }
        if let [p] = last {
            lanes[0] = step(lanes[0], u64::from(*p));
        }
    }
    lanes.iter().enumerate().fold(0, |h, (i, &lane)| h ^ step(lane, i as u64).rotate_left(16 * i as u32))
}

/// Whether `outer` holds all of `inner`.
pub fn contains(outer: &Rect, inner: &Rect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.w <= outer.x + outer.w
        && inner.y + inner.h <= outer.y + outer.h
}

/// The tiles of `area` (in whole tiles, clipped to the output) that `except` does not cover.
pub fn tiles_outside(area: &Rect, except: Option<&Rect>) -> Vec<Rect> {
    let (right, bottom) = (area.x + area.w, area.y + area.h);
    let columns = area.x / TILE..(right + TILE - 1) / TILE;
    (area.y / TILE..(bottom + TILE - 1) / TILE)
        .flat_map(|row| columns.clone().map(move |column| tile_rect(column, row, right, bottom)))
        .filter(|tile| !except.is_some_and(|e| contains(e, tile)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_region_is_the_box_of_recently_changed_tiles() {
        let mut activity = Activity::new(1280, 800);
        let start = Instant::now();
        assert_eq!(activity.region(start), None);
        // A change inside one tile and another two tiles further: their box, in whole tiles.
        activity.mark(&[Rect::new(70, 70, 10, 10), Rect::new(200, 130, 1, 1)], start);
        assert_eq!(activity.region(start), Some(Rect::new(64, 64, 192, 128)));
        // A change that spans tiles marks each.
        activity.mark(&[Rect::new(60, 60, 8, 8)], start);
        assert_eq!(activity.region(start), Some(Rect::new(0, 0, 256, 192)));
        // Older changes leave the region.
        let later = start + ACTIVE_WINDOW;
        activity.mark(&[Rect::new(640, 320, 64, 64)], later);
        assert_eq!(activity.region(later), Some(Rect::new(640, 320, 64, 64)));
        assert_eq!(activity.region(later + ACTIVE_WINDOW), None);
    }

    #[test]
    fn a_clipped_edge_tile_takes_its_neighbour() {
        // 800 rows: the last tile row holds 32.
        let mut activity = Activity::new(1280, 800);
        let now = Instant::now();
        activity.mark(&[Rect::new(10, 790, 5, 5)], now);
        assert_eq!(activity.region(now), Some(Rect::new(0, 704, 64, 96)));
        let mut narrow = Activity::new(1300, 800);
        narrow.mark(&[Rect::new(1290, 10, 5, 5)], now);
        assert_eq!(narrow.region(now), Some(Rect::new(1216, 0, 84, 64)));
    }

    #[test]
    fn only_continuous_motion_is_sustained() {
        let mut activity = Activity::new(1280, 800);
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        // A video's tile, changing every 40 ms; a hover's transition of 300 ms; a caret blinking every 500 ms.
        for ms in (0..=600).step_by(40) {
            activity.mark(&[Rect::new(0, 0, 64, 64)], at(ms));
        }
        for ms in (300..=600).step_by(20) {
            activity.mark(&[Rect::new(640, 320, 64, 64)], at(ms));
        }
        for ms in [0, 500] {
            activity.mark(&[Rect::new(1216, 736, 1, 1)], at(ms));
        }
        assert_eq!(activity.moving(at(600)), Some(Rect::new(0, 0, 64, 64)));
        assert_eq!(activity.region(at(600)), Some(Rect::new(0, 0, 1280, 768)), "every change counts for the region");
        // Motion that stopped stays sustained for the active window, then is gone.
        assert_eq!(activity.moving(at(1000)), Some(Rect::new(0, 0, 64, 64)));
        assert_eq!(activity.moving(at(1100)), None);
        // A pause longer than the gap starts the motion over.
        for ms in (1100..=1400).step_by(40) {
            activity.mark(&[Rect::new(0, 0, 64, 64)], at(ms));
        }
        assert_eq!(activity.moving(at(1400)), None);
    }

    #[test]
    fn observed_tiles_change_only_when_their_pixels_do() {
        let (width, height) = (200, 130);
        let mut activity = Activity::new(width, height);
        let mut pixels = vec![0u32; (width * height) as usize];
        let start = Instant::now();
        let all = [Rect::new(0, 0, width, height)];
        activity.observe(&pixels, &all, start);
        assert_eq!(activity.region(start), None, "the output starts black");
        // One pixel, damaged by a small rectangle and by a large overlapping one: its whole tile, once.
        pixels[(70 * width + 70) as usize] = 1;
        activity.observe(&pixels, &[Rect::new(70, 70, 1, 1), Rect::new(0, 0, 150, 100)], start);
        assert_eq!(activity.region(start), Some(Rect::new(64, 64, 64, 64)));
        // Damage over unchanged pixels leaves the time of the last change.
        let later = start + Duration::from_millis(300);
        activity.observe(&pixels, &all, later);
        assert!(activity.settled(&Rect::new(64, 64, 64, 64), start - ACTIVE_WINDOW, Duration::from_millis(300), later));
        // The last pixel of the clipped corner tile, and a change no damage reports, which is not looked at.
        pixels[(width * height - 1) as usize] = 7;
        pixels[0] = 9;
        activity.observe(&pixels, &[Rect::new(width - 1, height - 1, 1, 1)], later);
        assert_eq!(activity.region(later), Some(Rect::new(64, 64, 136, 66)));
        // A tile marked by hand (a failed lossless trial) keeps its time through an unchanged observation.
        let marked = Rect::new(128, 0, 64, 64);
        activity.mark(&[marked], later);
        let next = later + Duration::from_millis(10);
        activity.observe(&pixels, &[marked], next);
        assert!(activity.settled(&marked, later - Duration::from_millis(1), Duration::from_millis(10), next));
    }

    #[test]
    fn one_changed_pixel_always_changes_the_hash() {
        let seed = 0x1234;
        let mut tile = vec![0u32; 64 * 64];
        let first = hash(seed, tile.chunks(64));
        for (i, value) in [(0, 1), (63, 0x8000_0000), (64 * 64 - 1, 0xFF00_0000), (2000, 0xFFFF_FFFF)] {
            tile[i] = value;
            assert_ne!(hash(seed, tile.chunks(64)), first, "pixel {i}");
            tile[i] = 0;
        }
        // An edge tile's leftover pixels count too, also as pairs.
        let odd = vec![0u32; 13];
        for change in [[(12, 1), (12, 1)], [(8, 0x1235), (9, 0)], [(8, 0x01ea_ade4), (9, 0xae78_d11f)]] {
            let mut changed = odd.clone();
            change.iter().for_each(|&(i, v)| changed[i] = v);
            assert_ne!(hash(seed, std::iter::once(&odd[..])), hash(seed, std::iter::once(&changed[..])));
        }
    }

    #[test]
    fn a_tile_that_stopped_after_sustained_motion_settles_after_a_longer_rest() {
        let mut activity = Activity::new(1280, 800);
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let (tile, quiet) = (Rect::new(0, 0, 64, 64), Duration::from_millis(500));
        // A transition of a whole second, then still.
        for ms in (0..=1000).step_by(40) {
            activity.mark(&[tile], at(ms));
        }
        assert!(!activity.settled(&tile, start, quiet, at(2000)), "a brief change would have settled by now");
        assert!(activity.settled(&tile, start, quiet, at(1000) + SETTLED_AFTER_MOTION));
        // An overlay that goes right after it is repaired as late, not never.
        activity.mark(&[tile], at(1100));
        assert!(!activity.settled(&tile, start, quiet, at(1700)));
        assert!(activity.settled(&tile, start, quiet, at(1100) + SETTLED_AFTER_MOTION));
    }

    #[test]
    fn only_a_change_of_many_tiles_at_once_settles_a_large_area() {
        let mut activity = Activity::new(1280, 800);
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let (whole, quiet) = (Rect::new(0, 0, 1280, 768), Duration::from_millis(500));
        // Figures crossing a video: 40 tiles, one after another, each still once passed (more than the 24 needed).
        for (i, ms) in (0..4000).step_by(100).enumerate() {
            activity.mark(&[Rect::new(i as i32 % 20 * 64, 384 + i as i32 / 20 * 64, 1, 1)], at(ms));
        }
        assert!(!activity.settled(&whole, start, quiet, at(4500)), "40 tiles of 240, one at a time");
        // ... until the first of them has been still for SETTLED_ALONE_AFTER.
        assert!(!activity.settled(&whole, start, quiet, at(5000)));
        assert!(activity.settled(&whole, start, quiet, at(100) + SETTLED_ALONE_AFTER));
        // A panel of 6 x 4 tiles changes at once: a tenth of the area.
        activity.mark(&[Rect::new(320, 64, 384, 256)], at(4500));
        assert!(!activity.settled(&whole, at(4000), quiet, at(4900)), "not still long enough");
        assert!(activity.settled(&whole, at(4000), quiet, at(5000)));
    }

    #[test]
    fn a_tile_that_changed_and_then_stayed_still_has_settled() {
        let mut activity = Activity::new(1280, 800);
        let start = Instant::now();
        let area = Rect::new(0, 0, 256, 256);
        let quiet = Duration::from_millis(500);
        // Four of its sixteen tiles change at once.
        activity.mark(&[Rect::new(10, 10, 100, 100)], start);
        assert!(!activity.settled(&area, start - quiet, quiet, start), "it changed just now");
        assert!(activity.settled(&area, start - quiet, quiet, start + quiet));
        assert!(!activity.settled(&area, start, quiet, start + quiet), "not after the last keyframe");
        assert!(!activity.settled(&Rect::new(256, 0, 64, 64), start - quiet, quiet, start + quiet), "another tile");
    }

    #[test]
    fn what_a_region_would_leave_is_still_only_when_nothing_there_changed() {
        let mut activity = Activity::new(1280, 800);
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let (outer, inner, second) = (Rect::new(0, 0, 256, 256), Rect::new(0, 0, 128, 128), Duration::from_secs(1));
        activity.mark(&[Rect::new(0, 0, 256, 256)], start);
        activity.mark(&[inner], at(900));
        assert!(!activity.still_outside(&outer, &inner, second, at(900)));
        assert!(activity.still_outside(&outer, &inner, second, at(1000)), "only the inner part changed since");
        activity.mark(&[Rect::new(200, 200, 1, 1)], at(1500));
        assert!(!activity.still_outside(&outer, &inner, second, at(2000)), "one tile outside changed");
    }

    #[test]
    fn the_tiles_a_region_leaves_behind() {
        let old = Rect::new(0, 704, 192, 96);
        let new = Rect::new(64, 704, 128, 96);
        assert_eq!(tiles_outside(&old, Some(&new)), [Rect::new(0, 704, 64, 64), Rect::new(0, 768, 64, 32)]);
        assert_eq!(tiles_outside(&new, Some(&old)), []);
        assert_eq!(tiles_outside(&new, None).len(), 4);
        assert!(contains(&old, &new) && !contains(&new, &old));
    }
}
