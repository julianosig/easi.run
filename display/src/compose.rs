//! Software composition into a compositor-owned framebuffer.
//!
//! A commit only records the newly attached buffer and its damage. At the next frame, each damaged surface's pixels
//! are copied out of the client's pool (see [`crate::shm`]) into the surface's own buffer, and the scene is
//! recomposed into the framebuffer only where something changed: new content, a surface that moved, appeared, or
//! disappeared. Each surface is therefore read at most once per frame however fast a client commits, and every
//! pixel that leaves the compositor comes from compositor-owned memory.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{
    BufferAssignment, Damage, SubsurfaceCachedState, SurfaceAttributes, TraversalAction, with_states,
    with_surface_tree_upward,
};
use wayland_server::protocol::{wl_buffer::WlBuffer, wl_output};
use wayland_server::{Resource, backend::ObjectId};

use crate::limits;
use crate::shm;
use crate::state::{Accounting, ClientState};

/// A rectangle in output or surface pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// The right and bottom edges, computed wide: a client's rectangle may reach past `i32::MAX`.
    fn ends(&self) -> (i64, i64) {
        (self.x as i64 + self.w as i64, self.y as i64 + self.h as i64)
    }

    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let (x0, y0) = (self.x.max(other.x), self.y.max(other.y));
        let ((x1, y1), (other_x1, other_y1)) = (self.ends(), other.ends());
        let (x1, y1) = (x1.min(other_x1), y1.min(other_y1));
        // Both ends lie between two i32 values, so the sizes fit an i32 whenever the result is not empty.
        (x1 > x0 as i64 && y1 > y0 as i64).then(|| Rect::new(x0, y0, (x1 - x0 as i64) as i32, (y1 - y0 as i64) as i32))
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        let (x1, y1) = self.ends();
        x >= self.x as f64 && y >= self.y as f64 && x < x1 as f64 && y < y1 as f64
    }

    /// Whether this holds all of `inner`, a rectangle that is not empty.
    pub fn contains_rect(&self, inner: &Rect) -> bool {
        self.intersect(inner) == Some(*inner)
    }

    /// The smallest rectangle holding both (rectangles of the output, whose ends fit an i32).
    pub fn union(&self, other: &Rect) -> Rect {
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        Rect::new(x, y, (self.x + self.w).max(other.x + other.w) - x, (self.y + self.h).max(other.y + other.h) - y)
    }
}

/// A rectangle of the output on the wire; the output's sides fit a `u16`.
impl From<Rect> for display_protocol::Rect {
    fn from(r: Rect) -> display_protocol::Rect {
        display_protocol::Rect { x: r.x as u16, y: r.y as u16, w: r.w as u16, h: r.h as u16 }
    }
}

/// The bytes of a surface's copy, counted against its client's socket until the copy goes (resized, removed, or the
/// surface destroyed with the copy).
struct Charge {
    accounting: Arc<Accounting>,
    bytes: usize,
}

impl Charge {
    /// Counts `bytes` against the socket, or `None` when that would take it past `limit`.
    fn take(accounting: &Arc<Accounting>, bytes: usize, limit: usize) -> Option<Charge> {
        let charge = Charge { accounting: accounting.clone(), bytes };
        (accounting.pixel_bytes.fetch_add(bytes, Ordering::Relaxed) + bytes <= limit).then_some(charge)
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.accounting.pixel_bytes.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

/// A surface's own copy of its pixels.
#[derive(Default)]
struct Content {
    /// Attached at the last commit and not yet copied; released as soon as it is copied or replaced.
    pending: Option<WlBuffer>,
    width: i32,
    height: i32,
    opaque: bool,
    pixels: Vec<u32>,
    charge: Option<Charge>,
    /// Surface-local rectangles to copy from `pending`.
    damage: Vec<Rect>,
    /// Surface-local rectangles copied in by the frame being drawn, to recompose. Only [`copy_pending`] adds to it, for
    /// a surface that frame draws (or the cursor), and the same frame takes them all, so it never holds more than
    /// [`limits::MAX_DAMAGE_RECTS`]. A surface's size change or unmap adds nothing: a surface drawn in the last frame
    /// then takes another rectangle in the scene, or leaves it, which recomposes its old place (see
    /// [`Scene::compose`]); one that was not drawn has no old place to recompose.
    changed: Vec<Rect>,
}

impl Content {
    fn mapped(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// Drops the pending buffer (released) and the copy, leaving the surface unmapped.
    fn unmap(&mut self) {
        if let Some(previous) = self.pending.take() {
            previous.release();
        }
        self.width = 0;
        self.height = 0;
        self.pixels = Vec::new();
        self.charge = None;
        self.damage.clear();
    }
}

type ContentCell = Mutex<Content>;

fn with_content<T>(surface: &WlSurface, f: impl FnOnce(&mut Content) -> T) -> T {
    with_states(surface, |states| {
        states.data_map.insert_if_missing_threadsafe(ContentCell::default);
        f(&mut states.data_map.get::<ContentCell>().expect("inserted").lock().unwrap())
    })
}

fn push_rect(list: &mut Vec<Rect>, rect: Rect, whole: Rect) {
    let Some(rect) = rect.intersect(&whole) else { return };
    if list.len() >= limits::MAX_DAMAGE_RECTS {
        list.clear();
        list.push(whole);
    } else {
        list.push(rect);
    }
}

pub struct Scene {
    pub width: i32,
    pub height: i32,
    /// XRGB8888, row-major, `width * height`. Shared with the stream's snapshots instead of copied into them.
    pub framebuffer: Arc<Vec<u32>>,
    /// Earlier framebuffers, each with the composition it holds, reused once no snapshot holds them any more.
    spare: Vec<(Arc<Vec<u32>>, u64)>,
    /// Compositions painted so far: the framebuffer holds the last.
    painted: u64,
    /// What the last few compositions painted, each with its number, oldest first: a reused spare is brought up to
    /// date by copying only these from the framebuffer.
    history: VecDeque<(u64, Vec<Rect>)>,
    /// Output rectangles recomposed by the last frame, consumed by the encoder and capture.
    pub frame_damage: Vec<Rect>,
    /// Where each surface was drawn in the last frame, and in which order (bottom to top).
    placed: HashMap<ObjectId, Rect>,
    order: Vec<ObjectId>,
    /// Output damage not tied to a surface still in the scene (an unmap, a destroyed popup).
    loose: Vec<Rect>,
    pub frame_scheduled: bool,
}

impl Scene {
    pub fn new(width: i32, height: i32) -> Scene {
        Scene {
            width,
            height,
            framebuffer: Arc::new(vec![0; (width * height) as usize]),
            spare: Vec::new(),
            painted: 0,
            history: VecDeque::new(),
            frame_damage: Vec::new(),
            placed: HashMap::new(),
            order: Vec::new(),
            loose: Vec::new(),
            frame_scheduled: false,
        }
    }

    pub fn whole(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    pub fn is_mapped(&self, surface: &WlSurface) -> bool {
        surface.is_alive() && with_content(surface, |c| c.mapped() || c.pending.is_some())
    }

    /// Records a commit. Refuses scaled or transformed buffers, which this output never asks for.
    pub fn commit(&mut self, surface: &WlSurface) -> Result<(), &'static str> {
        let (assignment, damage, scale, transform) = with_states(surface, |states| {
            let mut cached = states.cached_state.get::<SurfaceAttributes>();
            let attrs = cached.current();
            (attrs.buffer.take(), std::mem::take(&mut attrs.damage), attrs.buffer_scale, attrs.buffer_transform)
        });
        if scale != 1 || transform != wl_output::Transform::Normal {
            return Err("scaled or transformed buffers are not supported");
        }
        with_content(surface, |content| {
            match assignment {
                Some(BufferAssignment::NewBuffer(buffer)) => {
                    let Some(data) = shm::buffer_data(&buffer) else {
                        return Err("only wl_shm buffers are supported");
                    };
                    let (w, h) = (data.width as i32, data.height as i32);
                    let opaque = data.opaque;
                    // The buffer still pending, attached again (with more damage), is read and released once, by the
                    // next frame: released now, the client would draw into it before that frame reads it.
                    if let Some(previous) = content.pending.replace(buffer)
                        && content.pending.as_ref() != Some(&previous)
                    {
                        previous.release();
                    }
                    if (w, h) != (content.width, content.height) {
                        // The old copy's bytes go before the new one's count.
                        content.charge = None;
                        content.pixels = Vec::new();
                        let limit = limits::MAX_PIXEL_OUTPUTS * (self.width * self.height) as usize * 4;
                        let accounting =
                            surface.client().and_then(|c| Some(c.get_data::<ClientState>()?.accounting.clone()));
                        let charge = accounting.and_then(|a| Charge::take(&a, (w * h) as usize * 4, limit));
                        let Some(charge) = charge else {
                            // The client is disconnected, but a frame may still read the surface (as the cursor).
                            content.unmap();
                            return Err("surface pixels exceed the socket's bound");
                        };
                        content.width = w;
                        content.height = h;
                        content.charge = Some(charge);
                        content.pixels = vec![0; (w * h) as usize];
                        content.damage = vec![Rect::new(0, 0, w, h)];
                    }
                    content.opaque = opaque;
                    let whole = Rect::new(0, 0, w, h);
                    for d in damage {
                        let r = match d {
                            Damage::Surface(r) => Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h),
                            Damage::Buffer(r) => Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h),
                        };
                        push_rect(&mut content.damage, r, whole);
                    }
                }
                Some(BufferAssignment::Removed) => content.unmap(),
                None => {}
            }
            Ok(())
        })
    }

    /// Copies pending client pixels into compositor memory. Returns the surfaces whose pool could not be read.
    fn pull(&self, scene: &[(WlSurface, Rect)]) -> Vec<WlSurface> {
        scene.iter().filter(|(surface, _)| !with_content(surface, copy_pending)).map(|(s, _)| s.clone()).collect()
    }

    /// Recomposes everything that changed since the last frame. `scene` lists the visible surfaces bottom to top with
    /// their output rectangles. Returns the surfaces to disconnect (their pool could not be read).
    pub fn compose(&mut self, scene: &[(WlSurface, Rect)]) -> Vec<WlSurface> {
        let broken = self.pull(scene);
        let whole = self.whole();
        let mut damage = std::mem::take(&mut self.loose);
        let mut now: HashMap<ObjectId, Rect> = HashMap::with_capacity(scene.len());
        for (surface, rect) in scene {
            let id = surface.id();
            match self.placed.get(&id) {
                Some(old) if old == rect => {}
                Some(old) => {
                    damage.push(*old);
                    damage.push(*rect);
                }
                None => damage.push(*rect),
            }
            with_content(surface, |content| {
                for r in content.changed.drain(..) {
                    damage.push(Rect::new(rect.x.saturating_add(r.x), rect.y.saturating_add(r.y), r.w, r.h));
                }
            });
            now.insert(id, *rect);
        }
        for (id, old) in &self.placed {
            if !now.contains_key(id) {
                damage.push(*old);
            }
        }
        // Surfaces restacked without new pixels (a subsurface placed above or below) show in their new order.
        let order: Vec<ObjectId> = scene.iter().map(|(s, _)| s.id()).collect();
        let kept = |ids: &[ObjectId], other: &HashMap<ObjectId, Rect>| {
            ids.iter().filter(|id| other.contains_key(id)).cloned().collect::<Vec<_>>()
        };
        let restacked = kept(&order, &self.placed);
        if kept(&self.order, &now) != restacked {
            damage.extend(restacked.iter().map(|id| now[id]));
        }
        self.placed = now;
        self.order = order;
        let mut clipped: Vec<Rect> = damage.iter().filter_map(|r| r.intersect(&whole)).collect();
        if clipped.len() > limits::MAX_DAMAGE_RECTS {
            clipped = clipped.into_iter().reduce(|a, b| a.union(&b)).into_iter().collect();
        }
        if !clipped.is_empty() {
            let width = self.width as usize;
            let fb = self.writable(&clipped);
            for region in &clipped {
                paint(fb, width, region, scene);
            }
        }
        self.frame_damage = clipped;
        broken
    }

    /// Damage for a surface that left the scene without a commit (a destroyed popup or toplevel).
    pub fn forget(&mut self, surface: &WlSurface) {
        if let Some(old) = self.placed.remove(&surface.id()) {
            self.loose.push(old);
        }
    }

    /// Drops the spare buffers no encoder still holds; whether any was dropped.
    pub fn release_spares(&mut self) -> bool {
        let before = self.spare.len();
        self.spare.retain(|(b, _)| Arc::strong_count(b) > 1);
        self.spare.len() < before
    }

    /// The framebuffer to paint `damage` into. In place while no snapshot holds it; otherwise a buffer no snapshot
    /// holds any more (or a new one), brought up to the current picture outside `damage`: a spare by copying what the
    /// compositions since its own painted, a new buffer, or a spare too far behind, by copying it all.
    fn writable(&mut self, damage: &[Rect]) -> &mut [u32] {
        if Arc::get_mut(&mut self.framebuffer).is_none() {
            let (mut next, age) = match self.spare.iter().position(|(b, _)| Arc::strong_count(b) == 1) {
                Some(i) => {
                    let (buffer, age) = self.spare.swap_remove(i);
                    (Arc::try_unwrap(buffer).expect("no other holder"), Some(age))
                }
                None => (vec![0; self.framebuffer.len()], None),
            };
            if damage != [self.whole()] {
                match age.and_then(|age| self.painted_since(age)) {
                    Some(rects) => copy_rects(&mut next, &self.framebuffer, self.width as usize, rects),
                    None => next.copy_from_slice(&self.framebuffer),
                }
            }
            let previous = std::mem::replace(&mut self.framebuffer, Arc::new(next));
            if self.spare.len() < 4 {
                self.spare.push((previous, self.painted));
            }
        }
        self.painted += 1;
        self.history.push_back((self.painted, damage.to_vec()));
        if self.history.len() > MAX_HISTORY {
            self.history.pop_front();
        }
        Arc::get_mut(&mut self.framebuffer).expect("held only here")
    }

    /// What the compositions after `age` painted, when the history still holds each of them and copying it is cheaper
    /// than copying the whole picture.
    fn painted_since(&self, age: u64) -> Option<impl Iterator<Item = &Rect>> {
        if self.history.front().is_some_and(|(first, _)| *first > age + 1) {
            return None;
        }
        let since = || self.history.iter().filter(move |(n, _)| *n > age).flat_map(|(_, rects)| rects);
        let area: i64 = since().map(|r| r.w as i64 * r.h as i64).sum();
        (area < self.width as i64 * self.height as i64).then(since)
    }
}

/// Compositions whose damage the scene remembers; a spare further behind is copied whole.
const MAX_HISTORY: usize = 8;

/// Copies `rects` of `src` into `dst`, both pictures `width` pixels wide.
pub fn copy_rects<'a>(dst: &mut [u32], src: &[u32], width: usize, rects: impl IntoIterator<Item = &'a Rect>) {
    for r in rects {
        for y in r.y as usize..(r.y + r.h) as usize {
            let row = y * width + r.x as usize..y * width + (r.x + r.w) as usize;
            dst[row.clone()].copy_from_slice(&src[row]);
        }
    }
}

/// Paints one output region of the scene into `fb`.
fn paint(fb: &mut [u32], width: usize, region: &Rect, scene: &[(WlSurface, Rect)]) {
    // Painting starts at the topmost opaque surface covering the region (nothing below it shows), else at the
    // lowest surface in the region. If that surface covers the region it is copied: a premultiplied pixel over the
    // black background is its own color. Otherwise the region is cleared and every surface blended.
    let covers = |rect: &Rect| rect.contains_rect(region);
    let opaque_cover =
        scene.iter().rposition(|(surface, rect)| covers(rect) && with_content(surface, |c| c.opaque && c.mapped()));
    let lowest = scene.iter().position(|(_, rect)| rect.intersect(region).is_some());
    let mut over_black = opaque_cover.or(lowest.filter(|&i| covers(&scene[i].1)));
    if over_black.is_none() {
        for y in region.y..region.y + region.h {
            let row = y as usize * width;
            fb[row + region.x as usize..row + (region.x + region.w) as usize].fill(0);
        }
    }
    let first = over_black.unwrap_or(0);
    for (surface, rect) in &scene[first..] {
        let Some(clip) = rect.intersect(region) else { continue };
        let copy = over_black.take().is_some();
        with_content(surface, |content| {
            if !content.mapped() {
                return;
            }
            let sw = content.width as usize;
            for y in clip.y..clip.y + clip.h {
                let src_row = (y - rect.y) as usize * sw;
                let src = &content.pixels[src_row + (clip.x - rect.x) as usize..][..clip.w as usize];
                let dst_row = y as usize * width + clip.x as usize;
                let dst = &mut fb[dst_row..dst_row + clip.w as usize];
                if content.opaque || copy {
                    for (d, s) in dst.iter_mut().zip(src) {
                        *d = s | 0xff00_0000;
                    }
                } else {
                    for (d, s) in dst.iter_mut().zip(src) {
                        *d = blend(*s, *d);
                    }
                }
            }
        });
    }
}

/// Copies a surface's pending buffer, if any, into its own pixels and releases it. False if the pool was unreadable.
fn copy_pending(content: &mut Content) -> bool {
    let Some(buffer) = content.pending.take() else { return true };
    let damage = std::mem::take(&mut content.damage);
    let ok = match shm::buffer_data(&buffer) {
        Some(data) => damage.iter().all(|r| {
            data.read_rect(r.x as u32, r.y as u32, r.w as u32, r.h as u32, &mut content.pixels, content.width as usize)
                .is_ok()
        }),
        None => false,
    };
    buffer.release();
    content.changed.extend(damage);
    ok
}

/// A cursor surface's size and straight-from-the-buffer pixels (premultiplied ARGB, opaque buffers with full alpha),
/// copied in from its pending buffer first. None while it has no pixels, if its pool is unreadable, or if either side
/// exceeds `max_side`: an oversized cursor's buffer is released unread. Cursor surfaces are not part of the scene, so
/// nothing else reads them. None for a destroyed surface, which the cursor status can still name.
pub fn cursor_pixels(surface: &WlSurface, max_side: i32) -> Option<(i32, i32, Vec<u32>)> {
    if !surface.is_alive() {
        return None;
    }
    with_content(surface, |content| {
        if content.width > max_side || content.height > max_side {
            if let Some(buffer) = content.pending.take() {
                buffer.release();
            }
            content.damage.clear();
            content.changed.clear();
            return None;
        }
        let ok = copy_pending(content);
        content.changed.clear();
        let alpha = if content.opaque { 0xff00_0000 } else { 0 };
        (ok && content.mapped())
            .then(|| (content.width, content.height, content.pixels.iter().map(|p| p | alpha).collect()))
    })
}

/// Premultiplied ARGB over opaque XRGB.
pub(crate) fn blend(src: u32, dst: u32) -> u32 {
    let a = src >> 24;
    if a == 0xff {
        return src;
    }
    if a == 0 && src & 0x00ff_ffff == 0 {
        return dst;
    }
    let inv = 255 - a;
    let channel = |shift: u32| {
        let s = (src >> shift) & 0xff;
        let d = (dst >> shift) & 0xff;
        (s + (d * inv + 127) / 255).min(255) << shift
    };
    0xff00_0000 | channel(16) | channel(8) | channel(0)
}

/// Lists a surface tree bottom to top with each mapped surface's output rectangle, `origin` being the root's position.
/// An unmapped surface hides its subsurfaces too, which keep their pixels for when it maps again.
pub fn surface_tree(root: &WlSurface, origin: (i32, i32), out: &mut Vec<(WlSurface, Rect)>) {
    type States = smithay::wayland::compositor::SurfaceData;
    let offset = |states: &States, parent: (i32, i32)| {
        if states.role == Some("subsurface") {
            let loc = states.cached_state.get::<SubsurfaceCachedState>().current().location;
            (parent.0.saturating_add(loc.x), parent.1.saturating_add(loc.y))
        } else {
            parent
        }
    };
    let size = |states: &States| {
        let size = states.data_map.get::<ContentCell>().map(|c| {
            let c = c.lock().unwrap();
            (c.width, c.height)
        });
        size.filter(|&(w, h)| w > 0 && h > 0)
    };
    with_surface_tree_upward(
        root,
        origin,
        |_, states, &parent| match size(states) {
            Some(_) => TraversalAction::DoChildren(offset(states, parent)),
            None => TraversalAction::SkipChildren,
        },
        |surface, states, &parent| {
            if let Some((w, h)) = size(states) {
                let (x, y) = offset(states, parent);
                out.push((surface.clone(), Rect::new(x, y, w, h)));
            }
        },
        |_, _, _| true,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blending_respects_premultiplied_alpha() {
        assert_eq!(blend(0xff11_2233, 0xff00_0000), 0xff11_2233);
        assert_eq!(blend(0x0000_0000, 0xff44_5566), 0xff44_5566);
        // Half-transparent black over white.
        assert_eq!(blend(0x8000_0000, 0xffff_ffff), 0xff7f_7f7f);
    }

    #[test]
    fn a_client_s_pixel_copies_are_bounded_and_counted_until_they_go() {
        let accounting = Arc::new(Accounting::default());
        let first = Charge::take(&accounting, 60, 100).expect("within the bound");
        assert!(Charge::take(&accounting, 60, 100).is_none(), "past the bound");
        assert_eq!(accounting.pixel_bytes.load(Ordering::Relaxed), 60, "a refused copy counts nothing");
        let second = Charge::take(&accounting, 40, 100).expect("up to the bound");
        drop(first);
        drop(second);
        assert_eq!(accounting.pixel_bytes.load(Ordering::Relaxed), 0);
    }

    /// Snapshots held for a while and released out of order leave spares of every age: each composition still holds
    /// the whole current picture, whatever spare it reused.
    #[test]
    fn a_reused_spare_holds_the_whole_current_picture() {
        let (w, h) = (96, 64);
        let mut scene = Scene::new(w, h);
        let mut expected = vec![0u32; (w * h) as usize];
        let mut held: Vec<Arc<Vec<u32>>> = Vec::new();
        let mut seed = 7u32;
        let mut next = |n: u32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 8) % n
        };
        for frame in 1..400u32 {
            let damage: Vec<Rect> = match next(20) {
                0 => vec![scene.whole()],
                1 => (0..limits::MAX_DAMAGE_RECTS as u32).map(|i| Rect::new((i % 12) as i32 * 8, 0, 8, 8)).collect(),
                _ => (0..1 + next(3))
                    .map(|_| Rect::new(next(80) as i32, next(48) as i32, 1 + next(16) as i32, 1 + next(16) as i32))
                    .collect(),
            };
            let fb = scene.writable(&damage);
            for r in &damage {
                for y in r.y..r.y + r.h {
                    for x in r.x..r.x + r.w {
                        fb[(y * w + x) as usize] = frame;
                        expected[(y * w + x) as usize] = frame;
                    }
                }
            }
            assert!(*scene.framebuffer == expected, "frame {frame}");
            // Viewers hold up to a few snapshots, for up to a dozen frames, and let go of any of them.
            if next(3) > 0 {
                held.push(scene.framebuffer.clone());
            }
            while held.len() > 1 + next(5) as usize {
                held.remove(next(held.len() as u32) as usize);
            }
        }
    }

    #[test]
    fn rectangles_intersect_and_bound() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(a.intersect(&Rect::new(5, 5, 10, 10)), Some(Rect::new(5, 5, 5, 5)));
        assert_eq!(a.intersect(&Rect::new(10, 0, 5, 5)), None);
        assert_eq!(Rect::new(1, 2, 3, 4).union(&Rect::new(10, 0, 1, 1)), Rect::new(1, 0, 10, 6));
        // A client's rectangles at the ends of i32 intersect without overflowing.
        let whole = Rect::new(0, 0, 1280, 800);
        assert_eq!(Rect::new(1, 0, i32::MAX, 1).intersect(&whole), Some(Rect::new(1, 0, 1279, 1)));
        assert_eq!(Rect::new(i32::MAX, 0, 100, 1).intersect(&whole), None);
        assert_eq!(Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX).intersect(&whole), None);
        assert_eq!(
            Rect::new(i32::MIN, 0, i32::MAX, 1).intersect(&Rect::new(i32::MIN, 0, 2, 1)),
            Some(Rect::new(i32::MIN, 0, 2, 1))
        );
        assert!(!Rect::new(i32::MAX, i32::MAX, i32::MAX, i32::MAX).contains(0.0, 0.0));
    }
}
