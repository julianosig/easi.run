//! Lossless tiles: the output cut into a fixed grid, the changed part of each damaged tile sent from compositor-owned
//! pixels as a solid, palette, or RGB record (whichever is smallest), the records of one message deflated together.

use display_protocol::{MAX_SERVER_MESSAGE, MAX_TILES, Message, Rect as WireRect, records};
use flate2::{Compress, Compression, FlushCompress, Status};

use crate::compose::Rect;

pub const TILE: i32 = 64;

/// The grid tiles a non-empty rectangle touches, as (column, row), on a grid of `columns` by `rows`.
pub fn tiles_of(rect: &Rect, columns: i32, rows: i32) -> impl Iterator<Item = (i32, i32)> {
    let (c0, r0) = ((rect.x / TILE).max(0), (rect.y / TILE).max(0));
    let c1 = ((rect.x + rect.w - 1) / TILE).min(columns - 1);
    let r1 = ((rect.y + rect.h - 1) / TILE).min(rows - 1);
    (r0..=r1).flat_map(move |r| (c0..=c1).map(move |c| (c, r)))
}
/// Tile (`column`, `row`) of the grid, clipped to `width` by `height`.
pub fn tile_rect(column: i32, row: i32, width: i32, height: i32) -> Rect {
    let (x, y) = (column * TILE, row * TILE);
    Rect::new(x, y, TILE.min(width - x), TILE.min(height - y))
}

/// Record bytes per tiles message: room for the headers and the rectangle table, and for deflate's worst-case growth
/// (a few bytes per stored block) on records that do not compress.
const BATCH_BYTES: usize = (MAX_SERVER_MESSAGE - 64 * 1024) / 64 * 63;
/// Slots of the color table that indexes a rectangle: twice the most colors a palette holds.
const SLOTS: usize = 512;

/// Which grid tiles still need sending.
#[derive(Clone, Debug)]
pub struct TileSet {
    columns: i32,
    rows: i32,
    dirty: Vec<bool>,
}

impl TileSet {
    pub fn new(width: i32, height: i32) -> TileSet {
        let columns = (width + TILE - 1) / TILE;
        let rows = (height + TILE - 1) / TILE;
        TileSet { columns, rows, dirty: vec![false; (columns * rows) as usize] }
    }

    pub fn mark(&mut self, rect: &Rect) {
        if rect.is_empty() {
            return;
        }
        for (c, r) in tiles_of(rect, self.columns, self.rows) {
            self.dirty[(r * self.columns + c) as usize] = true;
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.dirty.contains(&true)
    }

    /// Takes the dirty tiles as output rectangles, leaving the set empty.
    pub fn take(&mut self, width: i32, height: i32) -> Vec<Rect> {
        let mut out = Vec::new();
        for r in 0..self.rows {
            for c in 0..self.columns {
                let index = (r * self.columns + c) as usize;
                if std::mem::take(&mut self.dirty[index]) {
                    out.push(tile_rect(c, r, width, height));
                }
            }
        }
        out
    }
}

/// The pixels one viewer was last sent losslessly. A client's damage is often a bounding box around scattered changes
/// (a typed letter and a status line far apart), so damaged tiles whose pixels did not change are not sent again, and of
/// a tile that did change only the rectangle around its changed pixels is.
pub struct Sent {
    pixels: Vec<u32>,
    width: i32,
}

impl Sent {
    pub fn new(width: i32, height: i32) -> Sent {
        Sent { pixels: vec![0; (width * height) as usize], width }
    }

    /// Records a full lossless refresh: from now on the client shows exactly `pixels`.
    pub fn record_all(&mut self, pixels: &[u32]) {
        self.pixels.copy_from_slice(pixels);
    }

    /// Records `rects` of `pixels` as sent whole, whatever the client was sent before.
    pub fn record(&mut self, pixels: &[u32], rects: &[Rect]) {
        let width = self.width as usize;
        for r in rects {
            for y in r.y..r.y + r.h {
                let row = y as usize * width + r.x as usize..y as usize * width + (r.x + r.w) as usize;
                self.pixels[row.clone()].copy_from_slice(&pixels[row]);
            }
        }
    }

    /// Of each rectangle, the part around the pixels that differ from what the client was last sent (none when nothing
    /// differs), recorded as sent. Parts are not merged: one deflate stream carries all of a message's records, so a
    /// part costs only its rectangle and record tag, while a merged one would add unchanged pixels and turn cells of
    /// one color into palettes.
    pub fn changed(&mut self, pixels: &[u32], rects: Vec<Rect>) -> Vec<Rect> {
        rects.into_iter().filter_map(|r| self.changed_part(pixels, r)).collect()
    }

    fn changed_part(&mut self, pixels: &[u32], r: Rect) -> Option<Rect> {
        let width = self.width as usize;
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for y in r.y..r.y + r.h {
            let start = y as usize * width + r.x as usize;
            let (new, old) = (&pixels[start..start + r.w as usize], &self.pixels[start..start + r.w as usize]);
            if new == old {
                continue;
            }
            let first = new.iter().zip(old).position(|(a, b)| a != b).expect("the rows differ") as i32;
            let last = new.iter().zip(old).rposition(|(a, b)| a != b).expect("the rows differ") as i32;
            (x0, x1, y0, y1) = (x0.min(r.x + first), x1.max(r.x + last), y0.min(y), y);
        }
        if y1 < y0 {
            return None;
        }
        let part = Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1);
        self.record(pixels, &[part]);
        Some(part)
    }
}

/// The distinct colors of one rectangle and each pixel's index into them, in an open-addressing table that gives up
/// at the 257th color, so a photo costs a few hundred lookups rather than a full count.
struct Palette {
    keys: [u32; SLOTS],
    /// The color's index plus one, or zero for an empty slot.
    slots: [u16; SLOTS],
    colors: Vec<u32>,
    indices: Vec<u8>,
}

impl Palette {
    fn new() -> Palette {
        Palette { keys: [0; SLOTS], slots: [0; SLOTS], colors: Vec::with_capacity(256), indices: Vec::new() }
    }

    /// Indexes the rectangle's colors, or returns false when it has more than a palette holds.
    fn index(&mut self, pixels: &[u32], width: i32, rect: &Rect) -> bool {
        self.slots.fill(0);
        self.colors.clear();
        self.indices.clear();
        // Runs of one color, as interfaces are made of, are indexed once.
        for y in rect.y..rect.y + rect.h {
            let start = (y * width + rect.x) as usize;
            let row = &pixels[start..start + rect.w as usize];
            let mut at = 0;
            while at < row.len() {
                let color = row[at] & 0x00ff_ffff;
                let run = row[at..].iter().position(|&p| p & 0x00ff_ffff != color).unwrap_or(row.len() - at);
                let Some(index) = self.find(color) else { return false };
                self.indices.resize(self.indices.len() + run, index);
                at += run;
            }
        }
        true
    }

    fn find(&mut self, color: u32) -> Option<u8> {
        let mut slot = (color.wrapping_mul(0x9e37_79b1) >> 23) as usize;
        loop {
            match self.slots[slot] {
                0 if self.colors.len() == 256 => return None,
                0 => {
                    self.colors.push(color);
                    self.keys[slot] = color;
                    self.slots[slot] = self.colors.len() as u16;
                    return Some((self.colors.len() - 1) as u8);
                }
                index if self.keys[slot] == color => return Some((index - 1) as u8),
                _ => slot = (slot + 1) % SLOTS,
            }
        }
    }
}

/// The rectangle's color when every pixel has it, ahead of indexing. Each row folds its differences without an early
/// exit, so the comparison compiles to wide instructions; the rows stop at the first that differs.
fn solid(pixels: &[u32], width: i32, rect: &Rect) -> Option<u32> {
    let color = pixels[(rect.y * width + rect.x) as usize];
    let same = (rect.y..rect.y + rect.h).all(|y| {
        let start = (y * width + rect.x) as usize;
        pixels[start..start + rect.w as usize].iter().fold(0, |d, &p| d | (p ^ color)) & 0x00ff_ffff == 0
    });
    same.then_some(color & 0x00ff_ffff)
}

fn push_rgb(raw: &mut Vec<u8>, p: u32) {
    raw.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
}

/// Appends one rectangle of an XRGB8888 frame as its smallest record: solid when it holds one color, a palette when
/// that is smaller than RGB, else its RGB rows. Returns the RGB bytes it appended (zero for the other records).
fn push_record(raw: &mut Vec<u8>, palette: &mut Palette, pixels: &[u32], width: i32, rect: &Rect) -> usize {
    if let Some(color) = solid(pixels, width, rect) {
        raw.push(records::SOLID);
        push_rgb(raw, color);
        return 0;
    }
    if palette.index(pixels, width, rect) {
        let n = palette.colors.len();
        if records::palette_fits(n, palette.indices.len()) {
            raw.push(records::PALETTE);
            raw.push((n - 1) as u8);
            for &color in &palette.colors {
                push_rgb(raw, color);
            }
            raw.extend_from_slice(&palette.indices);
            return 0;
        }
    }
    raw.push(records::RGB);
    for y in rect.y..rect.y + rect.h {
        let row = (y * width) as usize;
        for &p in &pixels[row + rect.x as usize..row + (rect.x + rect.w) as usize] {
            push_rgb(raw, p);
        }
    }
    3 * (rect.w * rect.h) as usize
}

/// A raw-deflate compressor at the default level, which compresses antialiased text to about a third of QOI's size.
/// One is kept per viewer and reset for each message: a fresh one allocates and zeroes about 300 KB first.
pub fn compressor() -> Compress {
    Compress::new(Compression::default(), false)
}

/// One tiles message: the rectangles and their records, raw-deflated.
fn finish(deflate: &mut Compress, epoch: u32, revision: u64, rects: Vec<WireRect>, raw: &[u8]) -> Message {
    deflate.reset();
    let mut data = Vec::with_capacity(raw.len() / 2 + 64);
    loop {
        let read = deflate.total_in() as usize;
        match deflate.compress_vec(&raw[read..], &mut data, FlushCompress::Finish) {
            Ok(Status::StreamEnd) => break,
            Ok(_) => data.reserve(data.capacity()),
            Err(e) => unreachable!("deflating memory cannot fail: {e}"),
        }
    }
    Message::Tiles { epoch, revision, rects, raw_len: raw.len() as u32, data }
}

/// Encodes rectangles into as few tiles messages as the size bound allows, all of one revision. Gives up, before
/// deflating anything, once the rectangles that need RGB records hold more than `rgb_limit` bytes: content too rich
/// for a palette rarely deflates below a third of its size.
pub fn encode(
    deflate: &mut Compress,
    pixels: &[u32],
    width: i32,
    rects: &[Rect],
    epoch: u32,
    revision: u64,
    rgb_limit: usize,
) -> Option<Vec<Message>> {
    let mut palette = Palette::new();
    let mut batches = vec![(Vec::new(), Vec::new())];
    let mut rgb = 0;
    for rect in rects {
        let longest = 1 + 3 * (rect.w * rect.h) as usize;
        let (batch, raw) = batches.last().expect("one batch at least");
        if !batch.is_empty() && (raw.len() + longest > BATCH_BYTES || batch.len() == MAX_TILES) {
            batches.push((Vec::new(), Vec::new()));
        }
        let (batch, raw) = batches.last_mut().expect("one batch at least");
        rgb += push_record(raw, &mut palette, pixels, width, rect);
        if rgb > rgb_limit {
            return None;
        }
        batch.push((*rect).into());
    }
    Some(
        batches
            .into_iter()
            .filter(|(b, _)| !b.is_empty())
            .map(|(b, raw)| finish(deflate, epoch, revision, b, &raw))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_tiles_that_changed_since_the_last_send_go_out() {
        let (w, h) = (128, 64);
        let mut frame = vec![0xff00_0000u32; (w * h) as usize];
        let mut sent = Sent::new(w, h);
        sent.record_all(&frame);
        let both = vec![Rect::new(0, 0, 64, 64), Rect::new(64, 0, 64, 64)];
        assert!(sent.changed(&frame, both.clone()).is_empty(), "nothing changed");
        frame[(10 * w + 100) as usize] = 0xffff_ffff;
        assert_eq!(sent.changed(&frame, both.clone()), vec![Rect::new(100, 10, 1, 1)], "only the changed pixel");
        assert!(sent.changed(&frame, both.clone()).is_empty(), "the change was recorded as sent");
        // A change spread inside a tile goes out as the rectangle around it.
        frame[(3 * w + 70) as usize] = 0xff12_3456;
        frame[(40 * w + 90) as usize] = 0xff65_4321;
        assert_eq!(sent.changed(&frame, both), vec![Rect::new(70, 3, 21, 38)]);
    }

    #[test]
    fn marks_cover_the_touched_grid_cells() {
        let mut set = TileSet::new(1280, 800);
        set.mark(&Rect::new(63, 63, 2, 2));
        let tiles = set.take(1280, 800);
        assert_eq!(tiles.len(), 4);
        assert!(set.is_empty());
        set.mark(&Rect::new(0, 0, 1280, 800));
        let all = set.take(1280, 800);
        assert_eq!(all.len(), 20 * 13);
        // The last row is clipped to the output.
        assert_eq!(all.last().unwrap(), &Rect::new(1216, 768, 64, 32));
    }

    /// Inflates and decodes every message back into a frame of `width` x `height`.
    fn decode(messages: &[Message], width: i32, height: i32) -> Vec<u32> {
        let mut frame = vec![0u32; (width * height) as usize];
        for message in messages {
            let encoded = message.encode();
            assert!(encoded.len() <= MAX_SERVER_MESSAGE, "a tiles message fits the bound");
            let Message::Tiles { rects, raw_len, data, .. } = message else { panic!("a tiles message") };
            let mut raw = Vec::new();
            std::io::Read::read_to_end(&mut flate2::read::DeflateDecoder::new(&data[..]), &mut raw).unwrap();
            assert_eq!(raw.len(), *raw_len as usize);
            for (r, rgb) in rects.iter().zip(records::decode(rects, &raw).unwrap()) {
                for (i, px) in rgb.chunks(3).enumerate() {
                    let (x, y) = (r.x as i32 + i as i32 % r.w as i32, r.y as i32 + i as i32 / r.w as i32);
                    frame[(y * width + x) as usize] =
                        0xff00_0000 | u32::from(px[0]) << 16 | u32::from(px[1]) << 8 | u32::from(px[2]);
                }
            }
        }
        frame
    }

    #[test]
    fn each_rectangle_takes_its_smallest_record_and_round_trips() {
        let (w, h) = (192, 64);
        let noise = |i: u32| 0xff00_0000 | (i.wrapping_mul(2654435761) >> 8);
        // A solid tile, a two-color tile, and a noisy one.
        let pixels: Vec<u32> = (0..(w * h) as u32)
            .map(|i| match (i % w as u32) / 64 {
                0 => 0xff20_4060,
                1 if i % 3 == 0 => 0xffff_ffff,
                1 => 0xff00_0000,
                _ => noise(i),
            })
            .collect();
        let rects = [Rect::new(0, 0, 64, 64), Rect::new(64, 0, 64, 64), Rect::new(128, 0, 64, 64)];
        let messages = encode(&mut compressor(), &pixels, w, &rects, 1, 9, usize::MAX).unwrap();
        assert_eq!(messages.len(), 1);
        let Message::Tiles { raw_len, .. } = &messages[0] else { panic!() };
        assert_eq!(*raw_len as usize, 4 + (1 + 1 + 6 + 64 * 64) + (1 + 64 * 64 * 3), "solid, palette, RGB");
        assert_eq!(decode(&messages, w, h), pixels);
        // The noisy tile alone exceeds a smaller RGB allowance.
        assert!(encode(&mut compressor(), &pixels, w, &rects, 1, 9, 64 * 64 * 3 - 1).is_none());
    }

    #[test]
    fn a_large_refresh_splits_within_the_message_bound() {
        let (w, h) = (1920, 1080);
        let pixels: Vec<u32> = (0..(w * h) as u32).map(|i| 0xff00_0000 | (i.wrapping_mul(2654435761) >> 8)).collect();
        let mut set = TileSet::new(w, h);
        set.mark(&Rect::new(0, 0, w, h));
        let messages = encode(&mut compressor(), &pixels, w, &set.take(w, h), 1, 9, usize::MAX).unwrap();
        assert!(messages.len() > 1);
        assert_eq!(decode(&messages, w, h), pixels);
    }
}
