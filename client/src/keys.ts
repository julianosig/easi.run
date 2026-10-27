// `KeyboardEvent.code` (the physical key, layout-independent) to Linux evdev key codes (linux/input-event-codes.h).
// For keys sent by position (shortcuts, editing and navigation keys), the server's keymap turns the physical key into a
// character; ordinary typing goes as the characters themselves (`KeyKind.Text`), so any layout types what it shows.

const letters: Record<string, number> = {
  KeyQ: 16, KeyW: 17, KeyE: 18, KeyR: 19, KeyT: 20, KeyY: 21, KeyU: 22, KeyI: 23, KeyO: 24, KeyP: 25,
  KeyA: 30, KeyS: 31, KeyD: 32, KeyF: 33, KeyG: 34, KeyH: 35, KeyJ: 36, KeyK: 37, KeyL: 38,
  KeyZ: 44, KeyX: 45, KeyC: 46, KeyV: 47, KeyB: 48, KeyN: 49, KeyM: 50,
};

const digits: Record<string, number> = {
  Digit1: 2, Digit2: 3, Digit3: 4, Digit4: 5, Digit5: 6, Digit6: 7, Digit7: 8, Digit8: 9, Digit9: 10, Digit0: 11,
};

const punctuation: Record<string, number> = {
  Minus: 12, Equal: 13, BracketLeft: 26, BracketRight: 27, Semicolon: 39, Quote: 40, Backquote: 41,
  Backslash: 43, Comma: 51, Period: 52, Slash: 53, IntlBackslash: 86, IntlRo: 89, IntlYen: 124,
};

const editing: Record<string, number> = {
  Escape: 1, Backspace: 14, Tab: 15, Enter: 28, Space: 57, CapsLock: 58, NumLock: 69, ScrollLock: 70,
  PrintScreen: 99, Pause: 119, ContextMenu: 127,
};

const modifiers: Record<string, number> = {
  ControlLeft: 29, ShiftLeft: 42, AltLeft: 56, ShiftRight: 54, ControlRight: 97, AltRight: 100,
  MetaLeft: 125, MetaRight: 126,
};

const functions: Record<string, number> = {
  F1: 59, F2: 60, F3: 61, F4: 62, F5: 63, F6: 64, F7: 65, F8: 66, F9: 67, F10: 68, F11: 87, F12: 88,
};

const navigation: Record<string, number> = {
  Home: 102, ArrowUp: 103, PageUp: 104, ArrowLeft: 105, ArrowRight: 106, End: 107, ArrowDown: 108,
  PageDown: 109, Insert: 110, Delete: 111,
};

const keypad: Record<string, number> = {
  NumpadMultiply: 55, Numpad7: 71, Numpad8: 72, Numpad9: 73, NumpadSubtract: 74, Numpad4: 75, Numpad5: 76,
  Numpad6: 77, NumpadAdd: 78, Numpad1: 79, Numpad2: 80, Numpad3: 81, Numpad0: 82, NumpadDecimal: 83,
  NumpadEnter: 96, NumpadDivide: 98, NumpadEqual: 117, NumpadComma: 121,
};

const table: ReadonlyMap<string, number> = new Map(
  [letters, digits, punctuation, editing, modifiers, functions, navigation, keypad].flatMap((group) =>
    Object.entries(group),
  ),
);

/** The evdev code of a physical key, or undefined for a key with no mapping. */
export function evdevCode(code: string): number | undefined {
  return table.get(code);
}
