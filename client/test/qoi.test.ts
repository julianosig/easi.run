import assert from 'node:assert/strict';
import { test } from 'node:test';
import { decodeQoi, QoiError } from '../src/qoi.ts';

const END = [0, 0, 0, 0, 0, 0, 0, 1];

function qoi(width: number, height: number, ops: number[], options: { channels?: number; end?: number[] } = {}): Uint8Array {
  const header = [0x71, 0x6f, 0x69, 0x66, 0, 0, 0, 0, 0, 0, 0, 0, options.channels ?? 4, 0];
  const out = Uint8Array.from([...header, ...ops, ...(options.end ?? END)]);
  const view = new DataView(out.buffer);
  view.setUint32(4, width);
  view.setUint32(8, height);
  return out;
}

const pixels = (image: { rgba: Uint8Array }): number[][] => {
  const out: number[][] = [];
  for (let i = 0; i < image.rgba.length; i += 4) out.push(Array.from(image.rgba.subarray(i, i + 4)));
  return out;
};

const refuses = (bytes: Uint8Array, message: string, options = {}): void =>
  assert.throws(() => decodeQoi(bytes, options), (e: unknown) => e instanceof QoiError && e.message === message);

test('OP_RGB sets the color and keeps the previous alpha (initially 255)', () => {
  assert.deepEqual(pixels(decodeQoi(qoi(1, 1, [0xfe, 10, 20, 30], { channels: 3 }))), [[10, 20, 30, 255]]);
});

test('OP_RGBA sets every channel', () => {
  assert.deepEqual(pixels(decodeQoi(qoi(2, 1, [0xff, 1, 2, 3, 4, 0xfe, 9, 9, 9]))), [
    [1, 2, 3, 4],
    [9, 9, 9, 4],
  ]);
});

test('OP_INDEX reads the color hash table, which starts zeroed', () => {
  // (1 * 3 + 2 * 5 + 3 * 7 + 4 * 11) % 64 = 14.
  assert.deepEqual(pixels(decodeQoi(qoi(4, 1, [0x00, 0xff, 1, 2, 3, 4, 0xfe, 7, 7, 7, 0x0e]))), [
    [0, 0, 0, 0],
    [1, 2, 3, 4],
    [7, 7, 7, 4],
    [1, 2, 3, 4],
  ]);
});

test('OP_DIFF applies small differences with wraparound', () => {
  // dr = -2, dg = -1, db = +1 from (0, 0, 0, 255); then dr = dg = db = +1.
  assert.deepEqual(pixels(decodeQoi(qoi(2, 1, [0x47, 0x7f]))), [
    [254, 255, 1, 255],
    [255, 0, 2, 255],
  ]);
});

test('OP_LUMA applies a green difference and red and blue relative to it', () => {
  // dg = +10, dr - dg = -3, db - dg = +7; then dg = -32, dr - dg = -8, db - dg = +7 with wraparound.
  assert.deepEqual(pixels(decodeQoi(qoi(2, 1, [0xaa, 0x5f, 0x80, 0x0f]))), [
    [7, 10, 17, 255],
    [223, 234, 248, 255],
  ]);
});

test('OP_RUN repeats the previous pixel, from 1 up to 62 times', () => {
  assert.deepEqual(pixels(decodeQoi(qoi(4, 1, [0xfe, 5, 6, 7, 0xc2]))), [
    [5, 6, 7, 255],
    [5, 6, 7, 255],
    [5, 6, 7, 255],
    [5, 6, 7, 255],
  ]);
  const long = decodeQoi(qoi(63, 1, [0xfe, 1, 1, 1, 0xfd]));
  assert.deepEqual(pixels(long)[62], [1, 1, 1, 255]);
  // A run at the very start repeats the initial (0, 0, 0, 255).
  assert.deepEqual(pixels(decodeQoi(qoi(1, 2, [0xc1]))), [
    [0, 0, 0, 255],
    [0, 0, 0, 255],
  ]);
});

test('a multi-row image mixes every op', () => {
  const image = decodeQoi(qoi(3, 2, [0xff, 10, 20, 30, 40, 0xc0, 0x6e, 0xaa, 0x88, 0x00, 0x00]));
  assert.equal(image.width, 3);
  assert.equal(image.height, 2);
  assert.deepEqual(pixels(image), [
    [10, 20, 30, 40],
    [10, 20, 30, 40],
    [10, 21, 30, 40],
    [20, 31, 40, 40],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
  ]);
});

test('the end marker must follow the last pixel and end the input', () => {
  refuses(qoi(1, 1, [0xfe, 1, 2, 3], { end: [0, 0, 0, 0, 0, 0, 0, 2] }), 'end marker');
  refuses(qoi(1, 1, [0xfe, 1, 2, 3], { end: [] }), 'truncated');
  refuses(qoi(1, 1, [0xfe, 1, 2, 3, 0xfe, 1, 2, 3]), 'trailing bytes');
  refuses(Uint8Array.from([...qoi(1, 1, [0xfe, 1, 2, 3]), 0]), 'trailing bytes');
});

test('truncated streams are refused', () => {
  refuses(qoi(2, 1, [0xfe, 1, 2, 3]), 'truncated');
  refuses(qoi(1, 1, [0xfe, 1, 2]), 'truncated');
  refuses(qoi(1, 1, [0xff, 1, 2, 3]), 'truncated');
  refuses(qoi(1, 1, [0xaa]), 'truncated');
  refuses(Uint8Array.from([0x71, 0x6f, 0x69, 0x66, 0, 0]), 'truncated');
});

test('a run past the last pixel is refused', () => {
  refuses(qoi(2, 1, [0xc2]), 'run past the last pixel');
});

test('headers outside the bounds are refused before decoding', () => {
  refuses(Uint8Array.from([0x71, 0x6f, 0x69, 0x67, ...qoi(1, 1, [0xc0]).subarray(4)]), 'magic');
  refuses(qoi(1, 1, [0xc0], { channels: 5 }), 'channels');
  const badColorspace = qoi(1, 1, [0xc0]);
  badColorspace[13] = 2;
  refuses(badColorspace, 'colorspace');
  refuses(qoi(0, 1, [0xc0]), 'empty image');
  refuses(qoi(4097, 4096, [0xc0]), 'too large');
  refuses(qoi(65536, 65536, [0xc0]), 'too large');
  refuses(qoi(8, 8, [0xc0]), 'too large', { maxPixels: 63 });
  // A header claiming far more pixels than its stream could encode is refused without allocating for them.
  refuses(qoi(4096, 4096, [0xfd]), 'truncated');
});
