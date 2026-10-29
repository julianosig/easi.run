import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  cameraRect,
  DEFAULT_LAYOUT,
  keptCorner,
  microphoneSupported,
  moveCamera,
  parseLayout,
  resizeCamera,
  SIDE_MAX,
  SIDE_MIN,
  wire,
} from '../src/recorder.ts';
import { awaitAtMost } from '../src/deadline.ts';
import type { Rect } from '../src/protocol.ts';

test('the camera is a square where it was put, at its side, kept inside the screen (as the display draws it)', () => {
  assert.deepEqual(cameraRect(1280, 800, DEFAULT_LAYOUT), { x: 947, y: 464, w: 307, h: 307 });
  const corner = { ...DEFAULT_LAYOUT, x: 1, y: 1, side: 0.4 };
  assert.deepEqual(cameraRect(1280, 800, corner), { x: 768, y: 288, w: 512, h: 512 });
  assert.deepEqual(cameraRect(1280, 400, corner), { x: 880, y: 0, w: 400, h: 400 }, 'no taller than the screen');
  assert.deepEqual(cameraRect(1280, 800, { ...DEFAULT_LAYOUT, x: 0, y: 0, side: 0.1 }), { x: 0, y: 0, w: 128, h: 128 });
});

test('moving the camera stops at the screen edges', () => {
  const far = moveCamera(1280, 800, DEFAULT_LAYOUT, 2, -2);
  assert.deepEqual(cameraRect(1280, 800, far), { x: 973, y: 0, w: 307, h: 307 });
  assert.deepEqual(moveCamera(0, 0, DEFAULT_LAYOUT, 1, 1), DEFAULT_LAYOUT, 'no screen yet: unchanged');
});

test('resizing keeps the corner toward the nearest edges, within the smallest and largest side, inside the screen', () => {
  const kept = keptCorner(1280, 800, DEFAULT_LAYOUT);
  assert.deepEqual(kept, { right: true, bottom: true }, 'bottom right grows toward the middle');
  assert.deepEqual(keptCorner(1280, 800, { ...DEFAULT_LAYOUT, x: 0, y: 0 }), { right: false, bottom: false });
  const edges = (r: Rect) => [r.x + r.w, r.y + r.h, r.w, r.h];
  const larger = cameraRect(1280, 800, resizeCamera(1280, 800, DEFAULT_LAYOUT, 0.32, kept));
  assert.deepEqual(edges(larger), [1254, 771, 410, 410]);
  const largest = cameraRect(1280, 800, resizeCamera(1280, 800, DEFAULT_LAYOUT, 1, kept));
  assert.deepEqual(edges(largest), [1254, 771, SIDE_MAX * 1280, SIDE_MAX * 1280]);
  const smallest = cameraRect(1280, 800, resizeCamera(1280, 800, DEFAULT_LAYOUT, 0, kept));
  assert.deepEqual(edges(smallest), [1254, 771, SIDE_MIN * 1280, SIDE_MIN * 1280]);
  // From the top-left corner of a short screen, the camera grows only as far as the bottom edge.
  const top = { ...DEFAULT_LAYOUT, x: 0, y: 0.1 };
  const fitted = resizeCamera(1280, 400, top, SIDE_MAX, keptCorner(1280, 400, top));
  assert.deepEqual(cameraRect(1280, 400, fitted), { x: 0, y: 40, w: 360, h: 360 });
  assert.deepEqual(resizeCamera(0, 0, DEFAULT_LAYOUT, 0.3, kept), DEFAULT_LAYOUT, 'no screen yet: unchanged');
});

test('a layout goes on the wire in the protocol steps, within its bounds', () => {
  assert.deepEqual(wire({ x: 0.5, y: 1, side: 0.32, radius: 100 }), { x: 5000, y: 10_000, side: 3200, radius: 100 });
  const out = wire({ ...DEFAULT_LAYOUT, side: 0.9, radius: 140 });
  assert.deepEqual([out.side, out.radius], [4000, 100]);
  assert.equal(wire({ ...DEFAULT_LAYOUT, side: 0.01 }).side, 1000);
});

test('a remembered layout is read back only when it is whole and valid', () => {
  assert.deepEqual(parseLayout(JSON.stringify(DEFAULT_LAYOUT)), DEFAULT_LAYOUT);
  assert.equal(parseLayout(null), null);
  assert.equal(parseLayout('{'), null);
  assert.equal(parseLayout(JSON.stringify({ ...DEFAULT_LAYOUT, x: 2 })), null);
  assert.equal(parseLayout(JSON.stringify({ ...DEFAULT_LAYOUT, side: 0.05 })), null);
  assert.equal(parseLayout(JSON.stringify({ ...DEFAULT_LAYOUT, side: undefined })), null);
  assert.equal(parseLayout(JSON.stringify({ ...DEFAULT_LAYOUT, radius: 101 })), null);
  assert.equal(parseLayout(JSON.stringify({ ...DEFAULT_LAYOUT, radius: 2.5 })), null);
});

test('the microphone is offered only where the browser can send it', () => {
  const g = globalThis as Record<string, unknown>;
  const saved = { processor: g.MediaStreamTrackProcessor, encoder: g.AudioEncoder };
  try {
    g.MediaStreamTrackProcessor = undefined;
    g.AudioEncoder = class {};
    assert.equal(microphoneSupported(), false, 'no track processor (Firefox, Safari)');
    g.MediaStreamTrackProcessor = class {};
    assert.equal(microphoneSupported(), true);
  } finally {
    g.MediaStreamTrackProcessor = saved.processor;
    g.AudioEncoder = saved.encoder;
  }
});

test('a wait given up late never reaches what came after the recording it waited for', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let late = 0;
  // The recording ended before its wait ran out: nothing late, however long after.
  let ended = () => {};
  const done = new Promise<void>((resolve) => (ended = resolve));
  const waited = awaitAtMost(done, 15_000, () => late++);
  ended();
  await waited;
  t.mock.timers.tick(15_000);
  assert.equal(late, 0);
  // One that never ends is given up at the time, once.
  const given = awaitAtMost(new Promise<void>(() => {}), 15_000, () => late++);
  t.mock.timers.tick(14_999);
  assert.equal(late, 0);
  t.mock.timers.tick(1);
  await given;
  assert.equal(late, 1);
});
