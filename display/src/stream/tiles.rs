//! Lossless tiles: the output cut into a fixed grid, the changed part of each damaged tile sent from compositor-owned
//! pixels as a solid, palette, or RGB record (whichever is smallest), the records of one message deflated together.

use std::collections::VecDeque;

use display_protocol::{MAX_TILES, Message, Rect as WireRect, records};
use libdeflater::{CompressionLvl, Compressor};

use crate::compose::{Rect, copy_rects};

pub const TILE: i32 = 64;

/// The grid of tiles over an output of `width` by `height`, its last column and row clipped to it, numbered row by row.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    pub width: i32,
    pub height: i32,
    columns: i32,
    rows: i32,
}

impl Grid {
    pub fn new(width: i32, height: i32) -> Grid {
        Grid { width, height, columns: (width + TILE - 1) / TILE, rows: (height + TILE - 1) / TILE }
    }

    /// The number of tiles.
    pub fn len(&self) -> usize {
        (self.columns * self.rows) as usize
    }

    /// The tiles a rectangle touches; none for an empty one.
    pub fn indices(&self, rect: &Rect) -> impl Iterator<Item = usize> + use<> {
        let (c0, r0) = ((rect.x / TILE).max(0), (rect.y / TILE).max(0));
        let (c1, r1) = if rect.is_empty() {
            (-1, -1)
        } else {
            (((rect.x + rect.w - 1) / TILE).min(self.columns - 1), ((rect.y + rect.h - 1) / TILE).min(self.rows - 1))
        };
        let columns = self.columns;
        (r0..=r1).flat_map(move |r| (c0..=c1).map(move |c| (r * columns + c) as usize))
    }

    /// Tile `index`, clipped to the output.
    pub fn rect(&self, index: usize) -> Rect {
        let index = index as i32;
        let (x, y) = (index % self.columns * TILE, index / self.columns * TILE);
        Rect::new(x, y, TILE.min(self.width - x), TILE.min(self.height - y))
    }
}

/// Record bytes per tiles message, before deflating: a large update leaves as many small messages, each acknowledged
/// and paced on its own (see `flight`), well inside the message bound however little they compress.
const BATCH_BYTES: usize = 64 * 1024;
/// The most records one message holds when its tiles must go together (see [`encode_whole`]): a screen of text, and
/// half of the largest message, whatever it deflates to.
const WHOLE_BYTES: usize = 1024 * 1024;
/// Slots of the color table that indexes a rectangle: twice the most colors a palette holds.
const SLOTS: usize = 512;

/// Rectangles owed to a viewer whole: recorded as sent (see [`Sent`]) but not sent yet, at most one per grid tile (the
/// box around what is owed there). They are served in the order they first became owed: a tile owed again keeps its
/// place, so no tile waits behind newer work for ever.
pub struct Backlog {
    grid: Grid,
    owed: Vec<Option<Rect>>,
    order: VecDeque<usize>,
}

impl Backlog {
    pub fn new(width: i32, height: i32) -> Backlog {
        let grid = Grid::new(width, height);
        Backlog { grid, owed: vec![None; grid.len()], order: VecDeque::new() }
    }

    /// Owes `rects`, each cut at the grid.
    pub fn push(&mut self, rects: impl IntoIterator<Item = Rect>) {
        for rect in rects {
            for i in self.grid.indices(&rect) {
                let Some(part) = rect.intersect(&self.grid.rect(i)) else { continue };
                self.owed[i] = Some(match self.owed[i] {
                    Some(owed) => owed.union(&part),
                    None => {
                        self.order.push_back(i);
                        part
                    }
                });
            }
        }
    }

    /// Everything owed, oldest first, leaving nothing owed.
    pub fn take(&mut self) -> Vec<Rect> {
        let owed = &mut self.owed;
        self.order.drain(..).filter_map(|i| owed[i].take()).collect()
    }

    /// Owes only what `keep` keeps.
    pub fn retain(&mut self, keep: impl Fn(&Rect) -> bool) {
        let owed = &mut self.owed;
        self.order.retain(|&i| {
            let kept = owed[i].is_some_and(|r| keep(&r));
            if !kept {
                owed[i] = None;
            }
            kept
        });
    }

    pub fn clear(&mut self) {
        self.retain(|_| false);
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// The cells owed.
    pub fn len(&self) -> usize {
        self.order.len()
    }
}

/// Which grid tiles still need sending.
#[derive(Clone, Debug)]
pub struct TileSet {
    grid: Grid,
    dirty: Vec<bool>,
}

impl TileSet {
    pub fn new(width: i32, height: i32) -> TileSet {
        let grid = Grid::new(width, height);
        TileSet { grid, dirty: vec![false; grid.len()] }
    }

    pub fn mark(&mut self, rect: &Rect) {
        for i in self.grid.indices(rect) {
            self.dirty[i] = true;
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.dirty.contains(&true)
    }

    /// Takes the dirty tiles as output rectangles, leaving the set empty.
    pub fn take(&mut self) -> Vec<Rect> {
        let grid = self.grid;
        self.dirty.iter_mut().enumerate().filter_map(|(i, dirty)| std::mem::take(dirty).then(|| grid.rect(i))).collect()
    }
}

/// The pixels one viewer was last sent losslessly. A client's damage is often a bounding box around scattered changes
/// (a typed letter and a status line far apart), so damaged tiles whose pixels did not change are not sent again, and
/// of a tile that did change only the rectangle around its changed pixels is.
pub struct Sent {
    pixels: Vec<u32>,
    width: i32,
}

impl Sent {
    pub fn new(width: i32, height: i32) -> Sent {
        Sent { pixels: vec![0; (width * height) as usize], width }
    }

    /// Records `rects` of `pixels` as sent whole, whatever the client was sent before.
    pub fn record(&mut self, pixels: &[u32], rects: &[Rect]) {
        copy_rects(&mut self.pixels, pixels, self.width as usize, rects);
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

/// A copy the client makes within its own picture: the pixels of `rect` drawn again `dy` rows lower (higher when
/// negative), over what was there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Move {
    pub rect: Rect,
    pub dy: i32,
}

/// The least rows a moved part spans.
const MOVE_ROWS: i32 = TILE;
/// The least pixels a move must put where they belong (pixels that were wrong in place) to be worth sending: less, and
/// the tiles alone are as cheap.
const MOVE_GAIN: usize = 16 * (TILE * TILE) as usize;
/// The columns of pixels read to guess the shift, spread over the damaged area.
const MOVE_PROBES: usize = 8;
/// The pixels of a probe column, top to bottom, that vote for a shift together.
const MOVE_WINDOW: usize = 12;
/// The shifts with the most votes that are checked against the pictures; a guess needs this many votes.
const MOVE_GUESSES: usize = 3;
const MOVE_VOTES: u32 = 8;
/// The places of a window in the old column that vote: those nearest to where it is now.
const MOVE_ECHOES: usize = 8;
/// One row in this many is compared in place to count what a move puts right.
const MOVE_SAMPLE: usize = 4;

/// The box around `dirty` on a screen `width` wide holding `len` pixels, when it is large enough to hold a move.
fn move_area(dirty: &[Rect], width: i32, len: usize) -> Option<Rect> {
    let screen = Rect::new(0, 0, width, (len / width as usize) as i32);
    let area = dirty.iter().filter_map(|r| r.intersect(&screen)).reduce(|a, b| a.union(&b));
    area.filter(|a| a.h >= 2 * MOVE_ROWS && a.w as usize * a.h as usize >= MOVE_GAIN)
}

/// The parts of `old` that moved up or down to make `new`, within the box around `dirty` (a page scrolled), on a
/// screen `width` wide. A guess is checked against the pictures row by row, so every pixel a move copies is the
/// pixel `new` has there.
fn moves(old: &[u32], new: &[u32], width: i32, dirty: &[Rect]) -> Vec<Move> {
    let Some(area) = move_area(dirty, width, old.len()) else { return Vec::new() };
    // The most voted shift that explains enough: the others are echoes of it (lines that look alike).
    shifts(old, new, width, &area)
        .into_iter()
        .map(|dy| explained(old, new, width, &area, dy))
        .find(|(gain, _)| *gain >= MOVE_GAIN)
        .map_or_else(Vec::new, |(_, moves)| moves)
}

/// Whether `new` is `old` scrolled, within the box around `dirty`: whether [`Sent::moved`] would find moves from one
/// to the other.
pub fn scrolled(old: &[u32], new: &[u32], width: i32, dirty: &[Rect]) -> bool {
    old.len() == new.len() && !moves(old, new, width, dirty).is_empty()
}

impl Sent {
    /// The parts of the picture the client has that moved up or down to make `pixels`, within the box around `dirty`
    /// (a page scrolled): each is copied in this record of the client's picture as the client will copy it. Every
    /// pixel a move copies was checked against `pixels`; whatever else changed still differs afterwards, and
    /// [`Sent::changed`] finds it.
    pub fn moved(&mut self, pixels: &[u32], dirty: &[Rect]) -> Vec<Move> {
        let moves = moves(&self.pixels, pixels, self.width, dirty);
        for m in &moves {
            self.copy(m);
        }
        moves
    }

    /// Copies a move's rows in this record, in the order that reads each source row before it is written.
    fn copy(&mut self, m: &Move) {
        let (width, r) = (self.width as usize, m.rect);
        let row = |y: i32| y as usize * width + r.x as usize..y as usize * width + (r.x + r.w) as usize;
        let rows: Box<dyn Iterator<Item = i32>> =
            if m.dy > 0 { Box::new((r.y..r.y + r.h).rev()) } else { Box::new(r.y..r.y + r.h) };
        for y in rows {
            self.pixels.copy_within(row(y), row(y + m.dy).start);
        }
    }
}

/// The shifts most probe columns agree on, the shortest first: each window of a column's pixels in the new
/// picture votes for the distance to each place it has in the old column. A window of one color says nothing.
fn shifts(old_picture: &[u32], pixels: &[u32], width: i32, area: &Rect) -> Vec<i32> {
    let (width, rows) = (width as usize, area.h as usize);
    let mut votes = vec![0u32; 2 * rows];
    let (mut column, mut old) = (Vec::with_capacity(rows), Vec::with_capacity(rows));
    for probe in 0..MOVE_PROBES {
        let x = area.x as usize + (2 * probe + 1) * area.w as usize / (2 * MOVE_PROBES);
        // A window's hash, or none when it is of one color.
        let key = |column: &[u32], y: usize| {
            let window = &column[y..y + MOVE_WINDOW];
            let hash =
                window.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| (h ^ u64::from(*p)).wrapping_mul(0x0100_0000_01b3));
            window.iter().any(|p| *p != window[0]).then_some(hash)
        };
        let read = |picture: &[u32], column: &mut Vec<u32>| {
            column.clear();
            column.extend((0..rows).map(|y| picture[(area.y as usize + y) * width + x]));
        };
        read(old_picture, &mut column);
        old.clear();
        old.extend((0..=rows - MOVE_WINDOW).filter_map(|y| Some((key(&column, y)?, y))));
        old.sort_unstable();
        read(pixels, &mut column);
        for y in 0..=rows - MOVE_WINDOW {
            let Some(k) = key(&column, y) else { continue };
            // Lines that look alike put a window in several places: the nearest few vote.
            let places =
                &old[old.partition_point(|(other, _)| *other < k)..old.partition_point(|(other, _)| *other <= k)];
            let near = places.partition_point(|(_, from)| *from < y);
            let nearest = &places[near.saturating_sub(MOVE_ECHOES / 2)..(near + MOVE_ECHOES / 2).min(places.len())];
            for (_, from) in nearest {
                if *from != y {
                    votes[rows + y - from] += 1;
                }
            }
        }
    }
    // The shifts with at least half the most votes: among the echoes of one scroll, the shortest explains the most
    // rows, so the shortest go first.
    let most = votes.iter().copied().max().unwrap_or(0);
    let mut ranked: Vec<i32> = votes
        .iter()
        .enumerate()
        .filter(|(_, n)| **n >= MOVE_VOTES && **n >= most / 2)
        .map(|(at, _)| at as i32 - rows as i32)
        .collect();
    ranked.sort_by_key(|dy| (dy.abs(), *dy));
    ranked.truncate(MOVE_GUESSES);
    ranked
}

/// What a shift of `dy` rows explains of `pixels` inside `area`, column of tiles by column: in each, the longest
/// run of rows that are exactly the old picture's rows `dy` above (at least [`MOVE_ROWS`] of them), then one move
/// for each group of neighbouring columns with the same run: what stays beside or inside a scrolled part (a header
/// over some columns, a scrollbar) shortens only the columns it lies in. With about how many pixels the moves put right (those
/// that were wrong in place, counted on one row in [`MOVE_SAMPLE`]).
fn explained(old: &[u32], pixels: &[u32], width: i32, area: &Rect, dy: i32) -> (usize, Vec<Move>) {
    let width = width as usize;
    // The rows of the area whose source rows lie in it too.
    let (top, bottom) = ((area.y).max(area.y + dy), (area.y + area.h).min(area.y + area.h + dy));
    let same = |x0: i32, x1: i32, y: i32, from: i32| {
        let (new, from) = (y as usize * width, from as usize * width);
        pixels[new + x0 as usize..new + x1 as usize] == old[from + x0 as usize..from + x1 as usize]
    };
    // Per column of tiles: its bounds and its longest run of moved rows (first, past the last).
    let mut runs: Vec<(i32, i32, i32, i32)> = Vec::new();
    let mut x0 = area.x;
    while x0 < area.x + area.w {
        let x1 = ((x0 / TILE + 1) * TILE).min(area.x + area.w);
        let (mut best, mut start) = ((0, 0), top);
        for y in top..=bottom {
            if y == bottom || !same(x0, x1, y, y - dy) {
                if y - start > best.1 - best.0 {
                    best = (start, y);
                }
                start = y + 1;
            }
        }
        runs.push((x0, x1, best.0, best.1));
        x0 = x1;
    }
    let mut moves = Vec::new();
    let mut total = 0;
    let mut group: Option<(i32, i32, i32, i32)> = None;
    let mut close = |group: Option<(i32, i32, i32, i32)>, moves: &mut Vec<Move>| {
        let Some((x0, x1, y0, y1)) = group else { return };
        let wrong = (y0..y1).step_by(MOVE_SAMPLE).filter(|y| !same(x0, x1, *y, *y)).count();
        if wrong > 0 {
            total += wrong * MOVE_SAMPLE * (x1 - x0) as usize;
            moves.push(Move { rect: Rect::new(x0, y0 - dy, x1 - x0, y1 - y0), dy });
        }
    };
    for (x0, x1, y0, y1) in runs {
        match group {
            // The same rows as its neighbours: one move for them all.
            Some((gx0, _, gy0, gy1)) if (gy0, gy1) == (y0, y1) => group = Some((gx0, x1, y0, y1)),
            _ => {
                close(group.take(), &mut moves);
                group = (y1 - y0 >= MOVE_ROWS).then_some((x0, x1, y0, y1));
            }
        }
    }
    close(group, &mut moves);
    (total, moves)
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
        for row in rect.rows(pixels, width as usize) {
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
    let same =
        rect.rows(pixels, width as usize).all(|row| row.iter().fold(0, |d, &p| d | (p ^ color)) & 0x00ff_ffff == 0);
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
    for row in rect.rows(pixels, width as usize) {
        for &p in row {
            push_rgb(raw, p);
        }
    }
    3 * (rect.w * rect.h) as usize
}

/// A raw-deflate compressor (libdeflate) at the default level, which compresses antialiased text to about a third of
/// QOI's size: as small as zlib's default level, and four to seven times faster than miniz_oxide on screen tiles. Only
/// its compressor runs here, on records this module wrote. One is kept per viewer, with the buffer it writes into.
pub struct Deflate {
    compressor: Compressor,
    out: Vec<u8>,
}

pub fn compressor() -> Deflate {
    Deflate { compressor: Compressor::new(CompressionLvl::default()), out: Vec::new() }
}

/// One tiles message: the rectangles and their records, raw-deflated.
fn finish(deflate: &mut Deflate, epoch: u32, revision: u64, rects: Vec<WireRect>, raw: &[u8]) -> Message {
    // Written whole into a buffer of the bound libdeflate gives, which any input fits; the message keeps only its bytes.
    let bound = deflate.compressor.deflate_compress_bound(raw.len());
    if deflate.out.len() < bound {
        deflate.out.resize(bound, 0);
    }
    let written = deflate.compressor.deflate_compress(raw, &mut deflate.out).expect("the bound fits any input");
    Message::Tiles { epoch, revision, rects, raw_len: raw.len() as u32, data: deflate.out[..written].to_vec() }
}

/// A tiles or moved message's size on the wire: its fixed fields, its tables of moves and rectangles, and its records.
pub fn wire_bytes(message: &Message) -> usize {
    match message {
        Message::Tiles { rects, data, .. } => 19 + 8 * rects.len() + data.len(),
        Message::Moved { moves, rects, data, .. } => 21 + 12 * moves.len() + 8 * rects.len() + data.len(),
        _ => 0,
    }
}

/// Encodes rectangles, in order, into one tiles message of `revision`, whatever a message usually holds: the tiles
/// that go with moves are shown with them or not at all. None when they are more than a message may hold
/// ([`MAX_TILES`] rectangles) or than [`WHOLE_BYTES`] of records; no message for no rectangle.
pub fn encode_whole(
    deflate: &mut Deflate,
    pixels: &[u32],
    width: i32,
    rects: &[Rect],
    epoch: u32,
    revision: u64,
) -> Option<Option<Message>> {
    if rects.is_empty() {
        return Some(None);
    }
    if rects.len() > MAX_TILES {
        return None;
    }
    let mut palette = Palette::new();
    let mut raw = Vec::new();
    for rect in rects {
        push_record(&mut raw, &mut palette, pixels, width, rect);
        if raw.len() > WHOLE_BYTES {
            return None;
        }
    }
    Some(Some(finish(deflate, epoch, revision, rects.iter().map(|r| (*r).into()).collect(), &raw)))
}

/// The message that carries `moves` and then the tiles of `residue` (a tiles message of the same revision, or none
/// when the moves left nothing wrong).
pub fn moved(moves: &[Move], residue: Option<Message>, epoch: u32, revision: u64) -> Message {
    let moves = moves
        .iter()
        .map(|m| display_protocol::Move {
            rect: WireRect { x: m.rect.x as u16, y: m.rect.y as u16, w: m.rect.w as u16, h: m.rect.h as u16 },
            dx: 0,
            dy: m.dy as i16,
        })
        .collect();
    match residue {
        Some(Message::Tiles { rects, raw_len, data, .. }) => {
            Message::Moved { epoch, revision, moves, rects, raw_len, data }
        }
        _ => Message::Moved { epoch, revision, moves, rects: Vec::new(), raw_len: 0, data: Vec::new() },
    }
}

impl Move {
    /// Where its pixels land.
    pub fn target(&self) -> Rect {
        Rect::new(self.rect.x, self.rect.y + self.dy, self.rect.w, self.rect.h)
    }
}

/// Encodes rectangles, in order, into tiles messages of at most [`BATCH_BYTES`] of records each, the first of `revision`
/// and each next of the next revision, and stops after the message that brings them to `budget` bytes: the messages,
/// and how many of the rectangles they hold (the first ones). Gives up once the rectangles that need RGB records hold
/// more than `rgb_limit` bytes, before deflating the batch that crosses it (the full batches before it were deflated
/// already): content too rich for a palette rarely deflates below a third of its size.
#[allow(clippy::too_many_arguments)]
pub fn encode(
    deflate: &mut Deflate,
    pixels: &[u32],
    width: i32,
    rects: &[Rect],
    epoch: u32,
    revision: u64,
    rgb_limit: usize,
    budget: usize,
) -> Option<(Vec<Message>, usize)> {
    let mut palette = Palette::new();
    let (mut messages, mut batch, mut raw): (Vec<Message>, Vec<WireRect>, Vec<u8>) =
        (Vec::new(), Vec::new(), Vec::new());
    let (mut rgb, mut bytes) = (0, 0);
    for (i, rect) in rects.iter().enumerate() {
        let longest = 1 + 3 * (rect.w * rect.h) as usize;
        if !batch.is_empty() && (raw.len() + longest > BATCH_BYTES || batch.len() == MAX_TILES) {
            let message = finish(deflate, epoch, revision + messages.len() as u64, std::mem::take(&mut batch), &raw);
            raw.clear();
            bytes += wire_bytes(&message);
            messages.push(message);
            if bytes >= budget {
                return Some((messages, i));
            }
        }
        rgb += push_record(&mut raw, &mut palette, pixels, width, rect);
        if rgb > rgb_limit {
            return None;
        }
        batch.push((*rect).into());
    }
    if !batch.is_empty() {
        messages.push(finish(deflate, epoch, revision + messages.len() as u64, batch, &raw));
    }
    Some((messages, rects.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_tiles_that_changed_since_the_last_send_go_out() {
        let (w, h) = (128, 64);
        let mut frame = vec![0xff00_0000u32; (w * h) as usize];
        let mut sent = Sent::new(w, h);
        sent.record(&frame, &[Rect::new(0, 0, w, h)]);
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
        let tiles = set.take();
        assert_eq!(tiles.len(), 4);
        assert!(set.is_empty());
        set.mark(&Rect::new(0, 0, 1280, 800));
        let all = set.take();
        assert_eq!(all.len(), 20 * 13);
        // The last row is clipped to the output.
        assert_eq!(all.last().unwrap(), &Rect::new(1216, 768, 64, 32));
    }

    /// A page `rows` high of pixels that repeat nowhere.
    fn page(width: i32, rows: i32) -> Vec<u32> {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        (0..width * rows)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                0xff00_0000 | (state >> 40) as u32
            })
            .collect()
    }

    /// The screen showing `page` from its row `offset` on.
    fn view(page: &[u32], width: i32, height: i32, offset: i32) -> Vec<u32> {
        page[(offset * width) as usize..((offset + height) * width) as usize].to_vec()
    }

    /// A client's picture after `moves` and then the pixels of `rects` taken from `pixels`.
    fn follow(mut picture: Vec<u32>, width: i32, moves: &[Move], pixels: &[u32], rects: &[Rect]) -> Vec<u32> {
        for m in moves {
            let before = picture.clone();
            for y in m.rect.y..m.rect.y + m.rect.h {
                let from = (y * width + m.rect.x) as usize;
                let to = ((y + m.dy) * width + m.rect.x) as usize;
                picture[to..to + m.rect.w as usize].copy_from_slice(&before[from..from + m.rect.w as usize]);
            }
        }
        copy_rects(&mut picture, pixels, width as usize, rects);
        picture
    }

    #[test]
    fn a_scrolled_page_goes_as_one_move_and_the_rows_it_exposed() {
        let (w, h) = (640, 384);
        let whole = Rect::new(0, 0, w, h);
        let page = page(w, 2000);
        let mut sent = Sent::new(w, h);
        let mut shown = view(&page, w, h, 500);
        sent.record(&shown, &[whole]);
        // Down by 4 rows, down by 200, and back up by 37.
        for offset in [504, 704, 667] {
            let next = view(&page, w, h, offset);
            let grid = Grid::new(w, h);
            let tiles: Vec<Rect> = (0..grid.len()).map(|i| grid.rect(i)).collect();
            let moves = sent.moved(&next, &tiles);
            let by = (shown.len() as i32, offset);
            assert_eq!(moves.len(), 1, "one move for {by:?}: {moves:?}");
            assert_eq!(moves[0].rect.w, w);
            let rest = sent.changed(&next, tiles);
            let exposed: i32 = rest.iter().map(|r| r.w * r.h).sum();
            assert_eq!(exposed, w * (h - moves[0].rect.h), "only the exposed rows go as tiles: {rest:?}");
            shown = follow(shown, w, &moves, &next, &rest);
            assert!(shown == next, "the client's picture is the new one");
            assert!(sent.pixels == next, "and so is the record of it");
        }
    }

    #[test]
    fn what_stays_beside_a_scrolled_part_is_left_alone() {
        let (w, h) = (640, 384);
        let page = page(w, 2000);
        let fixed = self::page(w, h);
        // A header 48 rows high and a sidebar 128 wide stay; the rest scrolls.
        let screen = |offset: i32| {
            let mut pixels = view(&page, w, h, offset);
            for y in 0..h {
                for x in 0..w {
                    if y < 48 || x < 128 {
                        pixels[(y * w + x) as usize] = fixed[(y * w + x) as usize];
                    }
                }
            }
            pixels
        };
        let mut sent = Sent::new(w, h);
        let mut shown = screen(100);
        sent.record(&shown, &[Rect::new(0, 0, w, h)]);
        let next = screen(120);
        let grid = Grid::new(w, h);
        let tiles: Vec<Rect> = (0..grid.len()).map(|i| grid.rect(i)).collect();
        let moves = sent.moved(&next, &tiles);
        assert_eq!(moves, vec![Move { rect: Rect::new(128, 68, w - 128, h - 68), dy: -20 }]);
        let rest = sent.changed(&next, tiles);
        assert!(
            rest.iter().all(|r| r.y >= h - 20 && r.x >= 128),
            "only the exposed rows of the scrolled part: {rest:?}"
        );
        shown = follow(shown, w, &moves, &next, &rest);
        assert!(shown == next);
    }

    #[test]
    fn a_scrolled_page_of_lines_that_all_look_alike_moves_too() {
        let (w, h) = (640, 384);
        let whole = [Rect::new(0, 0, w, h)];
        let noise = page(w, 24);
        // Every line of 24 rows is the same: 14 rows of pixels, 10 blank.
        let lines = |offset: i32| -> Vec<u32> {
            (0..w * h)
                .map(|i| {
                    let row = (i / w + offset) % 24;
                    if row < 14 { noise[(row * w + i % w) as usize] } else { 0xffff_ffff }
                })
                .collect()
        };
        let mut sent = Sent::new(w, h);
        let mut shown = lines(0);
        sent.record(&shown, &whole);
        let next = lines(4);
        let moves = sent.moved(&next, &whole);
        assert_eq!(moves.len(), 1, "{moves:?}");
        assert_eq!(moves[0].dy, -4, "the shortest of the shifts that explain it");
        let rest = sent.changed(&next, whole.to_vec());
        shown = follow(shown, w, &moves, &next, &rest);
        assert!(shown == next && sent.pixels == next);
    }

    #[test]
    fn a_picture_that_did_not_move_has_no_move() {
        let (w, h) = (640, 384);
        let whole = [Rect::new(0, 0, w, h)];
        let first = view(&page(w, 2000), w, h, 0);
        let mut sent = Sent::new(w, h);
        sent.record(&first, &whole);
        assert!(sent.moved(&first, &whole).is_empty(), "the same picture");
        // Another picture altogether.
        let other: Vec<u32> = first.iter().map(|p| p ^ 0x00ff_ffff).rev().collect();
        assert!(sent.moved(&other, &whole).is_empty(), "nothing of the old picture is in the new one");
        // Lines that all look the same: every shift by a line explains the picture as well as none.
        let lines: Vec<u32> =
            (0..w * h).map(|i| if (i / w) % 24 < 14 { first[(i % w) as usize] } else { 0xffff_ffff }).collect();
        sent.record(&lines, &whole);
        assert!(sent.moved(&lines, &whole).is_empty(), "a still page of equal lines");
    }

    #[test]
    #[ignore = "a timing, not a check"]
    fn timing_of_a_move() {
        let (w, h) = (1920, 1080);
        let whole = [Rect::new(0, 0, w, h)];
        let page = page(w, 3000);
        for (name, step) in [("scroll by 4", 4), ("no relation", 1500)] {
            let mut sent = Sent::new(w, h);
            let mut took = Vec::new();
            for i in 0..40 {
                let offset = if step == 4 { i * 4 } else { (i % 2) * step };
                let next = view(&page, w, h, offset);
                if i == 0 {
                    sent.record(&next, &whole);
                    continue;
                }
                let started = std::time::Instant::now();
                let moves = sent.moved(&next, &whole);
                took.push(started.elapsed().as_micros());
                sent.record(&next, &whole);
                assert_eq!(moves.is_empty(), step != 4);
            }
            took.sort();
            eprintln!("TIMING {name}: median {} us, worst {} us", took[took.len() / 2], took[took.len() - 1]);
        }
    }

    /// Inflates and decodes every message back into a frame of `width` x `height`.
    fn decode(messages: &[Message], width: i32, height: i32) -> Vec<u32> {
        let mut frame = vec![0u32; (width * height) as usize];
        for message in messages {
            let encoded = message.encode();
            assert!(encoded.len() <= display_protocol::MAX_SERVER_MESSAGE, "a tiles message fits the bound");
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
        let (messages, used) = encode(&mut compressor(), &pixels, w, &rects, 1, 9, usize::MAX, usize::MAX).unwrap();
        assert_eq!((messages.len(), used), (1, 3));
        let Message::Tiles { raw_len, .. } = &messages[0] else { panic!() };
        assert_eq!(*raw_len as usize, 4 + (1 + 1 + 6 + 64 * 64) + (1 + 64 * 64 * 3), "solid, palette, RGB");
        assert_eq!(decode(&messages, w, h), pixels);
        // The noisy tile alone exceeds a smaller RGB allowance.
        assert!(encode(&mut compressor(), &pixels, w, &rects, 1, 9, 64 * 64 * 3 - 1, usize::MAX).is_none());
    }

    #[test]
    fn a_large_refresh_splits_within_the_message_bound() {
        let (w, h) = (1920, 1080);
        let pixels: Vec<u32> = (0..(w * h) as u32).map(|i| 0xff00_0000 | (i.wrapping_mul(2654435761) >> 8)).collect();
        let mut set = TileSet::new(w, h);
        set.mark(&Rect::new(0, 0, w, h));
        let tiles = set.take();
        let (messages, used) = encode(&mut compressor(), &pixels, w, &tiles, 1, 9, usize::MAX, usize::MAX).unwrap();
        assert!(messages.len() > 1 && used == tiles.len());
        assert_eq!(decode(&messages, w, h), pixels);
        // Each message of its own revision, in order.
        let revisions: Vec<u64> =
            messages.iter().map(|m| if let Message::Tiles { revision, .. } = m { *revision } else { 0 }).collect();
        assert!(revisions.iter().zip(9..).all(|(r, want)| *r == want), "{revisions:?}");
    }

    #[test]
    fn an_encode_stops_after_the_message_that_reaches_its_budget() {
        let (w, h) = (1280, 256);
        let pixels: Vec<u32> = (0..(w * h) as u32).map(|i| 0xff00_0000 | (i.wrapping_mul(2654435761) >> 8)).collect();
        let mut set = TileSet::new(w, h);
        set.mark(&Rect::new(0, 0, w, h));
        let tiles = set.take();
        // Noise: five tiles of RGB records a message, 61 KB each once deflated.
        let (messages, used) = encode(&mut compressor(), &pixels, w, &tiles, 1, 9, usize::MAX, 100_000).unwrap();
        let bytes: usize = messages.iter().map(wire_bytes).sum();
        assert_eq!(messages.len(), 2, "the second reached the budget");
        assert!((100_000..100_000 + BATCH_BYTES + 1024).contains(&bytes), "{bytes}");
        let held: usize =
            messages.iter().map(|m| if let Message::Tiles { rects, .. } = m { rects.len() } else { 0 }).sum();
        assert_eq!(used, held, "the messages hold the first {used} rectangles");
        // The rest goes in the next encode, at the next revisions.
        let (rest, _) = encode(&mut compressor(), &pixels, w, &tiles[used..], 1, 11, usize::MAX, usize::MAX).unwrap();
        let mut all = messages;
        all.extend(rest);
        assert_eq!(decode(&all, w, h), pixels);
    }

    #[test]
    fn a_backlog_holds_one_box_per_tile_in_the_order_it_was_owed() {
        let mut backlog = Backlog::new(200, 130);
        assert!(backlog.is_empty());
        backlog.push([Rect::new(70, 70, 4, 4), Rect::new(10, 10, 2, 2)]);
        // Owed again: the box grows and keeps its place; a rectangle across tiles is cut at the grid.
        backlog.push([Rect::new(80, 80, 2, 2), Rect::new(60, 0, 10, 4)]);
        assert_eq!(
            backlog.take(),
            [Rect::new(70, 70, 12, 12), Rect::new(10, 0, 54, 12), Rect::new(64, 0, 6, 4)],
            "in the order first owed"
        );
        assert!(backlog.is_empty());
        // What reaches past the output is clipped to its last tile; and what a region keeps.
        backlog.push([Rect::new(195, 129, 50, 50), Rect::new(0, 0, 64, 64)]);
        backlog.retain(|r| r.x > 0);
        assert_eq!(backlog.take(), [Rect::new(195, 129, 5, 1)]);
        backlog.push([Rect::new(0, 0, 1, 1)]);
        backlog.clear();
        assert!(backlog.is_empty() && backlog.take().is_empty());
    }
}
