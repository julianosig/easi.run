//! Linux evdev key codes (`linux/input-event-codes.h`), the code space of `Key.code`, for the keys the project names
//! itself: shortcuts it presses, names the agent may use, and keys the admin application watches for redaction.

pub const ESC: u16 = 1;
pub const BACKSPACE: u16 = 14;
pub const W: u16 = 17;
pub const TAB: u16 = 15;
pub const I: u16 = 23;
pub const ENTER: u16 = 28;
pub const J: u16 = 36;
pub const S: u16 = 31;
pub const LEFTCTRL: u16 = 29;
pub const LEFTSHIFT: u16 = 42;
pub const CAPSLOCK: u16 = 58;
pub const C: u16 = 46;
pub const V: u16 = 47;
pub const N: u16 = 49;
pub const RIGHTSHIFT: u16 = 54;
pub const LEFTALT: u16 = 56;
pub const SPACE: u16 = 57;
pub const F1: u16 = 59;
/// The ISO keyboard's extra key (xkb `<LSGT>`), absent from the US layout.
pub const KEY_102ND: u16 = 86;
pub const F11: u16 = 87;
pub const F12: u16 = 88;
pub const UP: u16 = 103;
pub const LEFT: u16 = 105;
pub const RIGHT: u16 = 106;
pub const DOWN: u16 = 108;
pub const LEFTMETA: u16 = 125;
/// The highest key code (`KEY_MAX`).
pub const MAX: u16 = 0x2ff;

/// The US layout's rows of characters from their first evdev key code, unshifted and shifted.
pub const US_ROWS: [(u16, &str, &str); 4] = [
    (2, "1234567890-=", "!@#$%^&*()_+"),
    (16, "qwertyuiop[]", "QWERTYUIOP{}"),
    (30, "asdfghjkl;'`", "ASDFGHJKL:\"~"),
    (44, "zxcvbnm,./", "ZXCVBNM<>?"),
];
