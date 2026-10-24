import assert from 'node:assert/strict';
import { test } from 'node:test';
import { decodeStamp, STAMP_HEIGHT, STAMP_WIDTH, unwrapStamp } from '../src/stamp.ts';

function strip(value: number, noise = 0): Uint8Array {
  const bits = [1, 0];
  for (let b = 39; b >= 0; b--) bits.push(Math.floor(value / 2 ** b) % 2);
  bits.push(0, 1);
  const rgba = new Uint8Array(STAMP_WIDTH * STAMP_HEIGHT * 4);
  for (let y = 0; y < STAMP_HEIGHT; y++) {
    for (let x = 0; x < STAMP_WIDTH; x++) {
      const level = bits[Math.floor(x / 16)] ? 255 - noise : noise;
      rgba.set([level, level, level, 255], (y * STAMP_WIDTH + x) * 4);
    }
  }
  return rgba;
}

test('decodes the time strip, also through lossy coding', () => {
  const now = 1_792_345_678_901;
  const low = decodeStamp(strip(now))!;
  assert.equal(low, now % 2 ** 40);
  assert.equal(decodeStamp(strip(now, 40)), low);
  // Drawn 37 ms before the client read it.
  assert.equal(unwrapStamp(low, now + 37), now);
});

test('refuses a strip without its sentinels or with an unreadable block', () => {
  const bad = strip(1234);
  bad.fill(128, (8 * STAMP_WIDTH + 5 * 16 + 8) * 4, (8 * STAMP_WIDTH + 5 * 16 + 9) * 4);
  assert.equal(decodeStamp(bad), null);
  assert.equal(decodeStamp(new Uint8Array(STAMP_WIDTH * STAMP_HEIGHT * 4)), null);
});
