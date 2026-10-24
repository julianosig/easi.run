import assert from 'node:assert/strict';
import { test } from 'node:test';
import { feedsComposition, HeldButtons, isApple, isModifier, isPasteShortcut, isShortcut, keyDown, keyText, optionAsControl, remoteKey, textChunks, toOutput, utf8Length, wheelPixels } from '../src/input.ts';
import { evdevCode } from '../src/keys.ts';
import { MAX_TEXT } from '../src/protocol.ts';

const screen = { width: 1280, height: 800 };

test('CSS pixels map to output pixels through the drawn size, clamped to the screen', () => {
  const box = { left: 100, top: 50, width: 640, height: 400 };
  assert.deepEqual(toOutput(100, 50, box, screen), { x: 0, y: 0 });
  assert.deepEqual(toOutput(420, 250, box, screen), { x: 640, y: 400 });
  assert.deepEqual(toOutput(739.9, 449.9, box, screen), { x: 1279, y: 799 });
  assert.deepEqual(toOutput(2000, -30, box, screen), { x: 1279, y: 0 });
  assert.deepEqual(toOutput(5, 5, { left: 0, top: 0, width: 0, height: 0 }, screen), { x: 1279, y: 799 });
  assert.deepEqual(toOutput(Number.NaN, 0, box, screen), { x: 0, y: 0 });
});

test('wheel deltas become pixels clamped to i16', () => {
  assert.equal(wheelPixels(-120, 0, 800), -120);
  assert.equal(wheelPixels(3, 1, 800), 48);
  assert.equal(wheelPixels(1, 2, 800), 800);
  assert.equal(wheelPixels(0.4, 0, 800), 0);
  assert.equal(wheelPixels(1e9, 0, 800), 0x7fff);
  assert.equal(wheelPixels(-1e9, 2, 800), -0x8000);
  assert.equal(wheelPixels(Number.POSITIVE_INFINITY, 0, 800), 0);
});

test('composed text splits on code points into pieces of at most 1024 bytes', () => {
  assert.deepEqual(textChunks(''), []);
  assert.deepEqual(textChunks('ção'), ['ção']);
  const long = '😀'.repeat(300) + 'a';
  const chunks = textChunks(long);
  assert.equal(chunks.join(''), long);
  assert(chunks.every((c) => c.length > 0 && utf8Length(c) <= MAX_TEXT));
  assert.deepEqual(chunks.map(utf8Length), [1024, 177]);
  assert.deepEqual(textChunks('abc', 2), ['ab', 'c']);
});

test('only characters travel as text, never named keys', () => {
  assert.equal(keyText('a'), 'a');
  assert.equal(keyText('Ω'), 'Ω');
  assert.equal(keyText('😀'), '😀');
  assert.equal(keyText('é'), 'é');
  for (const named of ['Enter', 'Dead', 'Unidentified', 'Process', 'F13', 'AudioVolumeUp', '']) {
    assert.equal(keyText(named), null, named);
  }
});

test("Ctrl, Meta, or Alt make a shortcut, but AltGr (Ctrl and Alt on Windows) and a Mac's Option alone type", () => {
  const press = (held: { ctrl?: boolean; meta?: boolean; alt?: boolean; altGraph?: boolean }) => ({
    ctrlKey: held.ctrl ?? false,
    metaKey: held.meta ?? false,
    altKey: held.alt ?? false,
    getModifierState: (key: string) => key === 'AltGraph' && (held.altGraph ?? false),
  });
  for (const apple of [false, true]) {
    assert.equal(isShortcut(press({}), apple), false);
    assert.equal(isShortcut(press({ ctrl: true }), apple), true);
    assert.equal(isShortcut(press({ meta: true }), apple), true);
    assert.equal(isShortcut(press({ ctrl: true, alt: true }), apple), true);
    assert.equal(isShortcut(press({ ctrl: true, alt: true, altGraph: true }), apple), false);
    assert.equal(isShortcut(press({ altGraph: true }), apple), false);
  }
  // Alt alone is a shortcut elsewhere, but a Mac's Option (reported as Alt, not AltGr) types the layout's characters.
  assert.equal(isShortcut(press({ alt: true }), false), true);
  assert.equal(isShortcut(press({ alt: true }), true), false);
  // Option with Command or Control is still a shortcut.
  assert.equal(isShortcut(press({ alt: true, meta: true }), true), true);
  assert.equal(isShortcut(press({ alt: true, ctrl: true }), true), true);
});

test('paste shortcuts are left to the paste event', () => {
  const base = { ctrlKey: false, metaKey: false, shiftKey: false, getModifierState: () => false };
  assert(isPasteShortcut({ ...base, code: 'KeyV', ctrlKey: true }, false));
  assert(isPasteShortcut({ ...base, code: 'Insert', shiftKey: true }, false));
  assert(!isPasteShortcut({ ...base, code: 'KeyV', metaKey: true }, false));
  assert(!isPasteShortcut({ ...base, code: 'KeyV' }, false));
  assert(!isPasteShortcut({ ...base, code: 'KeyC', ctrlKey: true }, false));
  // A Mac pastes with Command+V alone: its Control+V (and Shift+Insert) fire no paste, so they travel as keys.
  assert(isPasteShortcut({ ...base, code: 'KeyV', metaKey: true }, true));
  assert(!isPasteShortcut({ ...base, code: 'KeyV', ctrlKey: true }, true));
  assert(!isPasteShortcut({ ...base, code: 'Insert', shiftKey: true }, true));
  // AltGr+V types a character (Windows reports Control with it too).
  const altGraph = (key: string) => key === 'AltGraph';
  assert(!isPasteShortcut({ ...base, code: 'KeyV', ctrlKey: true, getModifierState: altGraph }, false));
});

test('a cancelled press releases the buttons still held, once', () => {
  const buttons = new HeldButtons();
  buttons.press(0);
  buttons.press(2);
  assert.deepEqual(buttons.cancel(), [0, 2]);
  assert.deepEqual(buttons.cancel(), []);
  // A release frees its button: the capture lost after it releases nothing more.
  buttons.press(0);
  buttons.release(0);
  assert.deepEqual(buttons.cancel(), []);
});

test('physical keys map to Linux evdev codes', () => {
  const expected: Record<string, number> = {
    Escape: 1, Digit1: 2, Digit0: 11, Minus: 12, Backspace: 14, Tab: 15, KeyQ: 16, KeyP: 25, Enter: 28,
    ControlLeft: 29, KeyA: 30, KeyL: 38, Quote: 40, Backquote: 41, ShiftLeft: 42, Backslash: 43, KeyZ: 44,
    KeyM: 50, Slash: 53, ShiftRight: 54, NumpadMultiply: 55, AltLeft: 56, Space: 57, CapsLock: 58, F1: 59,
    F10: 68, NumLock: 69, ScrollLock: 70, Numpad7: 71, NumpadSubtract: 74, NumpadAdd: 78, Numpad0: 82,
    NumpadDecimal: 83, IntlBackslash: 86, F11: 87, F12: 88, IntlRo: 89, NumpadEnter: 96, ControlRight: 97,
    NumpadDivide: 98, PrintScreen: 99, AltRight: 100, Home: 102, ArrowUp: 103, PageUp: 104, ArrowLeft: 105,
    ArrowRight: 106, End: 107, ArrowDown: 108, PageDown: 109, Insert: 110, Delete: 111, NumpadEqual: 117,
    Pause: 119, NumpadComma: 121, IntlYen: 124, MetaLeft: 125, MetaRight: 126, ContextMenu: 127,
    Equal: 13, BracketLeft: 26, BracketRight: 27, Semicolon: 39, Comma: 51, Period: 52, F5: 63, Numpad9: 73,
    Numpad5: 76, Numpad3: 81,
  };
  for (const [code, evdev] of Object.entries(expected)) assert.equal(evdevCode(code), evdev, code);
  assert.equal(evdevCode('Fn'), undefined);
  assert.equal(evdevCode(''), undefined);
  const codes = new Set<number>();
  for (const code of ['KeyA', 'KeyB', 'KeyC', 'KeyD', 'KeyE', 'KeyF', 'KeyG', 'KeyH', 'KeyI', 'KeyJ', 'KeyK', 'KeyL', 'KeyM',
    'KeyN', 'KeyO', 'KeyP', 'KeyQ', 'KeyR', 'KeyS', 'KeyT', 'KeyU', 'KeyV', 'KeyW', 'KeyX', 'KeyY', 'KeyZ']) {
    const evdev = evdevCode(code);
    assert(evdev !== undefined, code);
    codes.add(evdev);
  }
  assert.equal(codes.size, 26);
});

test('Command travels as Control from Apple platforms, every key as itself elsewhere', () => {
  assert.equal(remoteKey('MetaLeft', true), 'ControlLeft');
  assert.equal(remoteKey('MetaRight', true), 'ControlRight');
  assert.equal(remoteKey('KeyV', true), 'KeyV');
  assert.equal(remoteKey('ControlLeft', true), 'ControlLeft');
  assert.equal(remoteKey('MetaLeft', false), 'MetaLeft');
  assert(isApple('MacIntel') && isApple('iPad') && isApple('iPhone'));
  assert(!isApple('Linux x86_64') && !isApple('Win32') && !isApple(''));
});

test("a Mac's Option with an arrow, Backspace, or Delete travels as Control; with any other key it keeps Alt", () => {
  const press = (code: string, held: { ctrl?: boolean; meta?: boolean; alt?: boolean }) => ({
    code,
    ctrlKey: held.ctrl ?? false,
    metaKey: held.meta ?? false,
    altKey: held.alt ?? false,
  });
  for (const code of ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Backspace', 'Delete']) {
    assert.equal(optionAsControl(press(code, { alt: true }), true), true, code);
    // Elsewhere Alt is Alt, and without Option, or with Command or Control too, nothing is swapped.
    assert.equal(optionAsControl(press(code, { alt: true }), false), false, code);
    assert.equal(optionAsControl(press(code, {}), true), false, code);
    assert.equal(optionAsControl(press(code, { alt: true, meta: true }), true), false, code);
    assert.equal(optionAsControl(press(code, { alt: true, ctrl: true }), true), false, code);
  }
  for (const code of ['KeyL', 'Tab', 'Enter', 'Home', 'F5', 'Space']) {
    assert.equal(optionAsControl(press(code, { alt: true }), true), false, code);
  }
});

test('modifier keys are told apart from the keys they modify', () => {
  for (const code of ['ShiftLeft', 'ControlRight', 'AltLeft', 'MetaRight']) assert(isModifier(code), code);
  for (const code of ['KeyA', 'Enter', 'Meta', 'CapsLock']) assert(!isModifier(code), code);
});

test('a dead key or an IME press sends nothing; the character it composes arrives as text', () => {
  const press = (key: string, keyCode = 0, isComposing = false) => ({ key, keyCode, isComposing });
  assert.equal(feedsComposition(press('Dead', 222)), true);
  assert.equal(feedsComposition(press('Process', 229)), true);
  assert.equal(feedsComposition(press('a', 65, true)), true);
  assert.equal(feedsComposition(press('ã', 65)), false);
  assert.equal(feedsComposition(press('Enter', 13)), false);
});

test('a held key repeats from this browser, one press each, but a modifier or a lock key never does', () => {
  assert.equal(keyDown('Backspace', false, false), 'press');
  assert.equal(keyDown('Backspace', true, true), 'repeat');
  assert.equal(keyDown('ArrowLeft', true, true), 'repeat');
  // A repeat of a key not held remotely (control was granted while it was down) presses it.
  assert.equal(keyDown('Delete', true, false), 'press');
  // A press of a key held already lost its release: nothing more travels.
  assert.equal(keyDown('Backspace', false, true), 'none');
  for (const code of ['ShiftLeft', 'ControlRight', 'AltLeft', 'MetaRight', 'CapsLock', 'NumLock', 'ScrollLock']) {
    assert.equal(keyDown(code, false, false), 'press', code);
    assert.equal(keyDown(code, true, true), 'none', code);
    assert.equal(keyDown(code, true, false), 'none', code);
  }
});
