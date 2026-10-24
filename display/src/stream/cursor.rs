//! The pointer as viewers see it: hidden, one of the `wp_cursor_shape_device_v1` shapes Chromium names, or a custom
//! image read from the client's cursor surface, with the pointer's position for placing that image.

use std::sync::Arc;

use smithay::input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::with_states;

/// The protocol's cursor shape for a custom image.
pub const IMAGE: u8 = 255;
/// Largest custom image side, in pixels: the wire protocol allows a custom cursor image of at most 256x256.
const MAX_SIDE: i32 = 256;

/// The CSS names of shapes 1-34, in `wp_cursor_shape_device_v1` order; 0 is hidden.
const SHAPES: [&str; 34] = [
    "default",
    "context-menu",
    "help",
    "pointer",
    "progress",
    "wait",
    "cell",
    "crosshair",
    "text",
    "vertical-text",
    "alias",
    "copy",
    "move",
    "no-drop",
    "not-allowed",
    "grab",
    "grabbing",
    "e-resize",
    "n-resize",
    "ne-resize",
    "nw-resize",
    "s-resize",
    "se-resize",
    "sw-resize",
    "w-resize",
    "ew-resize",
    "ns-resize",
    "nesw-resize",
    "nwse-resize",
    "col-resize",
    "row-resize",
    "all-scroll",
    "zoom-in",
    "zoom-out",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Pointer {
    pub x: u16,
    pub y: u16,
    pub shape: u8,
    pub hot_x: u16,
    pub hot_y: u16,
    /// A QOI image (straight alpha) when `shape` is [`IMAGE`], else empty.
    pub image: Arc<[u8]>,
}

impl Pointer {
    /// Whether `other` looks the same, wherever it is.
    pub fn same_picture(&self, other: &Pointer) -> bool {
        (self.shape, self.hot_x, self.hot_y, &self.image) == (other.shape, other.hot_x, other.hot_y, &other.image)
    }
}

impl Default for Pointer {
    fn default() -> Pointer {
        Pointer { x: 0, y: 0, shape: 1, hot_x: 0, hot_y: 0, image: Arc::from([]) }
    }
}

/// A named shape's protocol number; shapes the protocol does not number show as the default arrow.
pub fn shape_number(icon: CursorIcon) -> u8 {
    SHAPES.iter().position(|name| *name == icon.name()).map_or(1, |i| i as u8 + 1)
}

/// The pointer for a cursor status at `(x, y)`. A surface cursor larger than the protocol allows, or one without
/// pixels yet, or whose encoded image exceeds the protocol's limit, shows as the default arrow.
pub fn pointer(status: &CursorImageStatus, (x, y): (u16, u16)) -> Pointer {
    let named = |shape| Pointer { x, y, shape, ..Pointer::default() };
    match status {
        CursorImageStatus::Hidden => named(0),
        CursorImageStatus::Named(icon) => named(shape_number(*icon)),
        CursorImageStatus::Surface(surface) => image(surface, (x, y)).unwrap_or_else(|| named(1)),
    }
}

fn image(surface: &WlSurface, (x, y): (u16, u16)) -> Option<Pointer> {
    let (width, height, pixels) = crate::compose::cursor_pixels(surface, MAX_SIDE)?;
    let hotspot = with_states(surface, |states| {
        states.data_map.get::<CursorImageSurfaceData>().map(|data| data.lock().unwrap().hotspot)
    })
    .unwrap_or_default();
    let rgba: Vec<u8> = pixels.iter().flat_map(|&p| straight_rgba(p)).collect();
    let image = qoi::encode_to_vec(&rgba, width as u32, height as u32).ok()?;
    // Clients refuse a larger image (a noisy one can exceed this well below 256x256).
    if image.len() > display_protocol::MAX_CURSOR_IMAGE {
        return None;
    }
    let side = |v: i32, max: i32| v.clamp(0, max - 1) as u16;
    Some(Pointer {
        x,
        y,
        shape: IMAGE,
        hot_x: side(hotspot.x, width),
        hot_y: side(hotspot.y, height),
        image: Arc::from(image),
    })
}

/// A premultiplied ARGB8888 pixel as straight-alpha RGBA bytes.
fn straight_rgba(pixel: u32) -> [u8; 4] {
    let a = pixel >> 24;
    let channel = |shift: u32| {
        let c = (pixel >> shift) & 0xff;
        (c * 255 + a / 2).checked_div(a).map_or(0, |v| v.min(255) as u8)
    };
    [channel(16), channel(8), channel(0), a as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_shapes_follow_the_protocol_order() {
        assert_eq!(shape_number(CursorIcon::Default), 1);
        assert_eq!(shape_number(CursorIcon::Pointer), 4);
        assert_eq!(shape_number(CursorIcon::Text), 9);
        assert_eq!(shape_number(CursorIcon::ZoomOut), 34);
        // Shapes after the protocol's 34 show as the default arrow.
        assert_eq!(shape_number(CursorIcon::AllResize), 1);
    }

    #[test]
    fn a_pointer_that_only_moved_keeps_its_picture() {
        let image = Pointer { shape: IMAGE, hot_x: 1, hot_y: 2, image: Arc::from([1u8, 2, 3]), ..Pointer::default() };
        assert!(image.same_picture(&Pointer { x: 9, y: 9, image: Arc::from([1u8, 2, 3]), ..image.clone() }));
        assert!(!image.same_picture(&Pointer { image: Arc::from([1u8, 2, 4]), ..image.clone() }));
        assert!(!image.same_picture(&Pointer { hot_x: 0, ..image.clone() }));
        assert!(!image.same_picture(&Pointer::default()));
    }

    #[test]
    fn premultiplied_pixels_become_straight_alpha() {
        assert_eq!(straight_rgba(0xff11_2233), [0x11, 0x22, 0x33, 0xff]);
        assert_eq!(straight_rgba(0x8040_2000), [0x80, 0x40, 0x00, 0x80]);
        assert_eq!(straight_rgba(0x0000_0000), [0, 0, 0, 0]);
    }
}
