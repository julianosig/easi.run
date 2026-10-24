import assert from 'node:assert/strict';
import { test } from 'node:test';

import { deflateSync } from 'fflate';

import { decodeTiles, inflateExact } from '../src/tile.ts';

const rgb = (w: number, h: number) => Uint8Array.from({ length: w * h * 3 }, (_, i) => (i * 37) & 255);
const at = (w: number, h: number) => ({ x: 0, y: 0, w, h });
const records = (...parts: number[][]) => Uint8Array.from(parts.flat());

/** Decodes records given as plain bytes, deflated as the server does. */
function decode(rects: { x: number; y: number; w: number; h: number }[], raw: Uint8Array) {
  return decodeTiles(rects, raw.length, deflateSync(raw));
}

test('each record kind decodes to RGBA with opaque alpha', () => {
  const pixels = rgb(2, 1);
  const raw = records([0, ...pixels], [1, 9, 8, 7], [2, 1, 10, 10, 10, 20, 20, 20, 0, 1, 1, 0, 0, 1]);
  const [plain, solid, palette] = decode([at(2, 1), at(2, 2), at(3, 2)], raw);
  assert.deepEqual([...plain!], [pixels[0], pixels[1], pixels[2], 255, pixels[3], pixels[4], pixels[5], 255]);
  assert.deepEqual([...solid!], [9, 8, 7, 255, 9, 8, 7, 255, 9, 8, 7, 255, 9, 8, 7, 255]);
  const [dark, light] = [[10, 10, 10, 255], [20, 20, 20, 255]];
  assert.deepEqual([...palette!], [dark, light, light, dark, dark, light].flat());
});

test('malformed records are refused', () => {
  const one = [at(2, 2)];
  assert.throws(() => decode(one, records([1, 1, 2])), /truncated/);
  assert.throws(() => decode(one, records([1, 1, 2, 3, 0])), /after the last one/);
  assert.throws(() => decode(one, records([7, 1, 2, 3])), /unknown tag/);
  assert.throws(() => decode(one, records([2, 1, 0, 0, 0, 1, 1, 1, 0, 2, 0, 0])), /index is out of range/);
  // Four colors for four pixels take more than RGB would.
  assert.throws(() => decode(one, records([2, 3, ...new Array(12).fill(0), 0, 1, 2, 3])), /invalid size/);
});

test('a stream longer or shorter than its declared length is refused', () => {
  assert.throws(() => inflateExact(deflateSync(rgb(5, 4)), 45), /past their length/);
  assert.throws(() => inflateExact(deflateSync(rgb(5, 2)), 45), /short of their length/);
});

test('a decompression bomb stops at the declared length', () => {
  const bomb = deflateSync(new Uint8Array(16 * 1024 * 1024), { level: 1 });
  assert(bomb.length < 128 * 1024);
  assert.throws(() => inflateExact(bomb, 64 * 64 * 3), /past their length/);
});

test('garbage, truncated streams, and empty tiles are refused', () => {
  assert.throws(() => inflateExact(Uint8Array.from([1, 2, 3, 4, 5]), 12));
  const stream = deflateSync(rgb(40, 40), { level: 0 });
  assert.throws(() => inflateExact(stream.subarray(0, stream.length - 10), 40 * 40 * 3));
  assert.throws(() => decodeTiles([at(0, 2)], 4, deflateSync(records([1, 0, 0, 0]))), /no pixels/);
});

test('a length or a tile larger than the protocol allows is refused before anything is allocated', () => {
  assert.throws(() => inflateExact(new Uint8Array(0), 8 * 1024 * 1024 + 1), /out of range/);
  assert.throws(() => decodeTiles([at(257, 1)], 4, deflateSync(records([1, 0, 0, 0]))), /larger than the protocol/);
});

test('a stream that ends early and is followed by garbage stops quickly', () => {
  const early = deflateSync(rgb(2, 2));
  const padded = new Uint8Array(early.length + 2 * 1024 * 1024);
  padded.set(early);
  const started = performance.now();
  assert.throws(() => inflateExact(padded, 64 * 64 * 3));
  assert(performance.now() - started < 500, 'refused without reading the padding slice by slice');
});

test('bytes after the last output beyond the stream end are refused at once', () => {
  const full = deflateSync(rgb(16, 16));
  const padded = new Uint8Array(full.length + 2 * 1024 * 1024);
  padded.set(full);
  const started = performance.now();
  assert.throws(() => inflateExact(padded, 16 * 16 * 3), /bytes after them/);
  assert(performance.now() - started < 500);
});

test('stored blocks, which output nothing until they are complete, decode', () => {
  const pixels = rgb(64, 64);
  const raw = records([0, ...pixels]);
  const [rgba] = decodeTiles([at(64, 64)], raw.length, deflateSync(raw, { level: 0 }));
  assert.deepEqual([...rgba!.subarray(0, 4)], [pixels[0], pixels[1], pixels[2], 255]);
  const big = rgb(256, 256);
  assert.equal(inflateExact(deflateSync(big, { level: 0 }), big.length).length, big.length);
});

test('a stream that outputs a few bytes and then a bomb is stopped by the bound', () => {
  const input = new Uint8Array(8 * 1024 * 1024);
  input.set(rgb(1, 1));
  const bomb = deflateSync(input, { level: 1 });
  const started = performance.now();
  assert.throws(() => inflateExact(bomb, 64 * 64 * 3), /past their length/);
  assert(performance.now() - started < 500);
});
