// Lossless tiles: one raw deflate stream per message, inflated under a hard bound on its output
// to exactly its declared length, then one record per rectangle, in order (see protocol/src/records.rs):
// - RGB (0): the rectangle's 8-bit RGB rows, top row first;
// - solid (1): one RGB color;
// - palette (2): the color count minus one (2 to 256 colors), the RGB colors, then one index byte per pixel.

import { Inflate } from 'fflate';

import { MAX_TILE_SIDE, MAX_TILES_RAW, type Rect } from './protocol.ts';

/** Input bytes fed to the inflater at a time: deflate expands at most about 1032 to 1, so no push outputs more than
 * about half a mebibyte before the bound is checked again. */
const SLICE = 512;
/** Input bytes in a row without output after which the stream counts as ended early: a stored block can hold
 * 65,535 bytes before it outputs any, and the inflater would otherwise keep copying input it no longer reads. */
const IDLE_BYTES = 66 * 1024;
/** Input bytes that may follow the last output byte: only the end of the final block, which is a few bytes. */
const END_BYTES = 64;

const RGB = 0;
const SOLID = 1;
const PALETTE = 2;

/** Inflates a raw deflate stream that must end with exactly `size` bytes. */
export function inflateExact(data: Uint8Array, size: number): Uint8Array {
  if (!Number.isInteger(size) || size < 0 || size > MAX_TILES_RAW) throw new Error('a records length out of range');
  const out = new Uint8Array(size);
  let filled = 0;
  let ended = false;
  const inflate = new Inflate((chunk, final) => {
    if (filled + chunk.length > size) throw new Error('tile records inflate past their length');
    out.set(chunk, filled);
    filled += chunk.length;
    ended ||= final;
  });
  // Every push is one bounded slice. Once the output is full, only the end of the stream may remain: more is refused
  // rather than fed to an inflater that would keep copying it.
  let at = 0;
  for (let idle = 0; at < data.length && filled < size; at += SLICE) {
    const before = filled;
    const slice = data.subarray(at, at + SLICE);
    inflate.push(slice, at + SLICE >= data.length);
    idle = filled === before ? idle + slice.length : 0;
    if (idle > IDLE_BYTES) throw new Error('a tile stream stops producing records');
  }
  if (!ended) {
    const rest = data.subarray(Math.min(at, data.length));
    if (rest.length > END_BYTES) throw new Error('tile records have bytes after them');
    inflate.push(rest, true);
  }
  if (!ended || filled !== size) throw new Error('tile records inflate short of their length');
  return out;
}

/** The RGB color at `raw[i]` as an RGBA pixel word. Browsers store a word's low byte first, so the word is
 * `A << 24 | B << 16 | G << 8 | R`. */
function word(raw: Uint8Array, i: number): number {
  return 0xff000000 | (raw[i + 2]! << 16) | (raw[i + 1]! << 8) | raw[i]!;
}

/** Decodes the records of `rects` to one RGBA image per rectangle. */
export function decodeTiles(rects: readonly Rect[], rawLen: number, data: Uint8Array): Uint8Array<ArrayBuffer>[] {
  const raw = inflateExact(data, rawLen);
  let at = 0;
  const take = (n: number): number => {
    if (at + n > raw.length) throw new Error('a tile record is truncated');
    const start = at;
    at += n;
    return start;
  };
  const images = rects.map(({ w, h }) => {
    if (!Number.isInteger(w) || !Number.isInteger(h) || w < 1 || h < 1) throw new Error('a tile has no pixels');
    if (w > MAX_TILE_SIDE || h > MAX_TILE_SIDE) throw new Error('a tile is larger than the protocol allows');
    const pixels = w * h;
    const rgba = new Uint8Array(pixels * 4);
    // Each pixel is written as one 32-bit word.
    const words = new Uint32Array(rgba.buffer);
    const tag = raw[take(1)];
    if (tag === RGB) {
      const src = take(3 * pixels);
      for (let i = src, p = 0; p < pixels; i += 3, p++) words[p] = word(raw, i);
    } else if (tag === SOLID) {
      words.fill(word(raw, take(3)));
    } else if (tag === PALETTE) {
      const n = raw[take(1)]! + 1;
      if (n < 2 || 1 + 3 * n + pixels > 3 * pixels) throw new Error('a tile palette has an invalid size');
      const colors = take(3 * n);
      const indices = take(pixels);
      // The palette as RGBA words.
      const table = new Uint32Array(n);
      for (let i = 0; i < n; i++) table[i] = word(raw, colors + 3 * i);
      for (let p = 0; p < pixels; p++) {
        const i = raw[indices + p]!;
        if (i >= n) throw new Error('a tile palette index is out of range');
        words[p] = table[i]!;
      }
    } else {
      throw new Error('a tile record has an unknown tag');
    }
    return rgba;
  });
  if (at !== raw.length) throw new Error('tile records have bytes after the last one');
  return images;
}
