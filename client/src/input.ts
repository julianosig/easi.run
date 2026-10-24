// Pure input mapping for control mode: coordinates, wheel deltas, and text bounds.

import { MAX_TEXT } from './protocol.ts';

interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** Maps a CSS-pixel position inside `box` (the drawn canvas) to output pixels, clamped to the screen. */
export function toOutput(
  clientX: number,
  clientY: number,
  box: Box,
  screen: { width: number; height: number },
): { x: number; y: number } {
  const scale = (v: number, start: number, size: number, max: number): number => {
    const mapped = Math.floor(((v - start) * max) / (size > 0 ? size : 1));
    return Math.min(max - 1, Math.max(0, Number.isFinite(mapped) ? mapped : 0));
  };
  return {
    x: scale(clientX, box.left, box.width, screen.width),
    y: scale(clientY, box.top, box.height, screen.height),
  };
}

const DOM_DELTA_LINE = 1;
const DOM_DELTA_PAGE = 2;
/** Pixels per line for `WheelEvent.DOM_DELTA_LINE` (Firefox's usual mode). */
const LINE_PIXELS = 16;

/** A wheel delta in pixels, rounded and clamped to i16; `page` is the pixel size of one page on this axis. */
export function wheelPixels(delta: number, mode: number, page: number): number {
  const pixels = mode === DOM_DELTA_LINE ? delta * LINE_PIXELS : mode === DOM_DELTA_PAGE ? delta * page : delta;
  const rounded = Math.round(pixels);
  return Number.isFinite(rounded) ? Math.min(0x7fff, Math.max(-0x8000, rounded)) : 0;
}

const encoder = new TextEncoder();

export function utf8Length(text: string): number {
  return encoder.encode(text).length;
}

/** Splits text into non-empty pieces of at most `max` UTF-8 bytes, never inside a code point. */
export function textChunks(text: string, max = MAX_TEXT): string[] {
  const chunks: string[] = [];
  let chunk = '';
  let bytes = 0;
  for (const point of text) {
    const size = utf8Length(point);
    if (bytes + size > max) {
      chunks.push(chunk);
      chunk = '';
      bytes = 0;
    }
    chunk += point;
    bytes += size;
  }
  if (chunk !== '') chunks.push(chunk);
  return chunks;
}

/**
 * The text a key produces when it has no physical-key mapping: its `KeyboardEvent.key` when that is a character,
 * never a named key such as `Enter`, `Dead`, or `Unidentified`.
 */
export function keyText(key: string): string | null {
  if (key === '') return null;
  if ([...key].length === 1) return key;
  return /^[A-Z][A-Za-z0-9]+$/.test(key) ? null : key;
}

/**
 * Whether a key press only feeds a composition, so nothing travels for it: an IME is composing (Safari reports some of
 * those presses only through keyCode 229), or it is a dead key (`~` or `´` on pt-BR ABNT2, US-International, French,
 * German). The composed character arrives as text afterwards, through the next key's `key` or `compositionend`; sending
 * the dead key's physical code too would type its own mark on the remote layout (`~` then `a` would give `'ã`).
 */
export function feedsComposition(e: { key: string; isComposing: boolean; keyCode: number }): boolean {
  return e.isComposing || e.keyCode === 229 || e.key === 'Dead';
}

/**
 * The physical key the remote browser gets for `code`. On a Mac, Command does there what Control does on Linux (copy,
 * paste, select all), so it travels as Control; elsewhere every key travels as itself.
 */
export function remoteKey(code: string, apple: boolean): string {
  if (!apple) return code;
  if (code === 'MetaLeft') return 'ControlLeft';
  if (code === 'MetaRight') return 'ControlRight';
  return code;
}

/**
 * Whether a key travels with Control in place of a Mac's Option. On a Mac, Option with an arrow moves by word or
 * paragraph and Option with Backspace or Delete deletes a word, which Control does on Linux, where Alt with Left or Right
 * goes Back or Forward instead. Option with any other key keeps Alt, and Option with a character types it.
 */
export function optionAsControl(
  e: { code: string; altKey: boolean; ctrlKey: boolean; metaKey: boolean },
  apple: boolean,
): boolean {
  return apple && e.altKey && !e.ctrlKey && !e.metaKey && /^(Arrow(Left|Right|Up|Down)|Backspace|Delete)$/.test(e.code);
}

/** Whether `code` names a modifier key (Shift, Control, Alt, Meta, either side). */
export function isModifier(code: string): boolean {
  return /^(Shift|Control|Alt|Meta)(Left|Right)$/.test(code);
}

/**
 * What a physical key's `keydown` sends: a press, a repeat (its release, then a press), or nothing. The remote seat
 * repeats no key, so every repeat of a held key travels from here, one press each: the admin then sees each character
 * a held Backspace deletes and each step a held arrow moves. The remote side takes a press of a key it holds down for
 * none, so a repeat releases the key first. Modifiers and lock keys never repeat, and a press of a key held already (its
 * release lost) sends nothing.
 */
export function keyDown(code: string, repeat: boolean, held: boolean): 'press' | 'repeat' | 'none' {
  const repeats = !isModifier(code) && !/^(CapsLock|NumLock|ScrollLock)$/.test(code);
  if (!held) return repeat && !repeats ? 'none' : 'press';
  return repeat && repeats ? 'repeat' : 'none';
}

/** Whether this browser runs on an Apple platform, whose shortcuts use Command. */
export function isApple(platform: string): boolean {
  return /^(Mac|iPhone|iPad|iPod)/.test(platform);
}

/**
 * Whether a key press is a shortcut (Ctrl, Meta, or Alt held), which travels as physical codes rather than as the
 * character it makes. AltGr is not: Windows reports it as Ctrl and Alt both held, yet it types characters. Nor is a
 * Mac's Option held alone: it is the layout's character modifier (`@` is Option+L on a German Mac).
 */
export function isShortcut(
  e: { ctrlKey: boolean; metaKey: boolean; altKey: boolean; getModifierState(key: string): boolean },
  apple: boolean,
): boolean {
  if (e.getModifierState('AltGraph')) return false;
  if (apple && e.altKey && !e.ctrlKey && !e.metaKey) return false;
  return e.ctrlKey || e.metaKey || e.altKey;
}

/**
 * True for the shortcuts that make the browser fire a `paste` event, which travels as a clipboard message instead.
 * AltGr+V is a character on some layouts (`@` on Czech and Slovak), whatever Control Windows reports with it.
 */
export function isPasteShortcut(e: {
  code: string;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  getModifierState(key: string): boolean;
}): boolean {
  if (e.getModifierState('AltGraph')) return false;
  return (e.code === 'KeyV' && (e.ctrlKey || e.metaKey)) || (e.code === 'Insert' && e.shiftKey);
}
