import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  KEYFRAME_RETRY_MS,
  MAX_REMEMBERED_TILES,
  OrderError,
  StreamOrder,
  subtract,
  subtractAll,
} from '../src/order.ts';
import { Codec, type MessageOf, type Rect, VERSION } from '../src/protocol.ts';

const W = 1280;
const H = 800;
const full: Rect = { x: 0, y: 0, w: W, h: H };

function config(epoch: number, codec: Codec = Codec.H264, width = W, height = H): MessageOf<'config'> {
  return { type: 'config', version: VERSION, epoch, width, height, codec, maxFps: 60 };
}

function video(epoch: number, revision: number, keyframe: boolean, rect: Rect = full): MessageOf<'video'> {
  return {
    type: 'video',
    epoch,
    revision: BigInt(revision),
    rect,
    keyframe,
    timestampUs: 0n,
    payload: new Uint8Array(1),
  };
}

function tiles(epoch: number, revision: number, ...rects: Rect[]): MessageOf<'tiles'> {
  return { type: 'tiles', epoch, revision: BigInt(revision), rects, rawLen: 4 * rects.length, data: new Uint8Array() };
}

function cursor(epoch: number): MessageOf<'cursor'> {
  return { type: 'cursor', epoch, x: 1, y: 1, shape: 1, hotX: 0, hotY: 0, image: new Uint8Array() };
}

/** Queues a frame and returns its sequence number. */
function queue(order: StreamOrder, m: MessageOf<'video'>, now = 0): number {
  const verdict = order.video(m, now);
  assert.equal(verdict.kind, 'decode');
  return verdict.kind === 'decode' ? verdict.seq : -1;
}

function area(rects: Rect[]): number {
  return rects.reduce((sum, r) => sum + r.w * r.h, 0);
}

function started(epoch = 1): StreamOrder {
  const order = new StreamOrder();
  assert.deepEqual(order.config(config(epoch), 0), { verdict: 'reset', keyframeRequest: epoch });
  return order;
}

test('rectangle subtraction yields disjoint parts of exactly the remaining area', () => {
  const a: Rect = { x: 0, y: 0, w: 100, h: 100 };
  assert.deepEqual(subtract(a, { x: 200, y: 0, w: 10, h: 10 }), [a]);
  assert.deepEqual(subtract(a, { x: 0, y: 0, w: 100, h: 100 }), []);
  assert.deepEqual(subtract(a, { x: -10, y: -10, w: 500, h: 500 }), []);
  const hole = subtract(a, { x: 10, y: 20, w: 30, h: 40 });
  assert.equal(hole.length, 4);
  assert.equal(area(hole), 100 * 100 - 30 * 40);
  for (const [i, p] of hole.entries()) {
    for (const q of hole.slice(i + 1)) {
      assert(subtract(p, q).length === 1 && subtract(p, q)[0] === p, 'parts overlap');
    }
  }
  const edge = subtract(a, { x: 50, y: 0, w: 50, h: 100 });
  assert.deepEqual(edge, [{ x: 0, y: 0, w: 50, h: 100 }]);
  assert.equal(area(subtractAll(full, [{ x: 0, y: 0, w: 64, h: 64 }, { x: 32, y: 32, w: 64, h: 64 }])), W * H - 64 * 64 * 2 + 32 * 32);
});

test('the first configuration resets and asks for a keyframe and refresh, as after a reconnect', () => {
  const order = new StreamOrder();
  assert.equal(order.screen, null);
  assert.deepEqual(order.config(config(4), 0), { verdict: 'reset', keyframeRequest: 4 });
  assert.equal(order.screen?.width, W);
  assert.equal(order.video(video(4, 1, false), 0).kind, 'drop');
  // A tiles-only configuration still asks: the answer is a full lossless refresh.
  const tilesOnly = new StreamOrder();
  assert.deepEqual(tilesOnly.config(config(9, Codec.None), 0), { verdict: 'reset', keyframeRequest: 9 });
});

test('a repeated configuration is a no-op; a changed one without a new epoch breaks the stream', () => {
  const order = started(2);
  assert.deepEqual(order.config(config(2), 0), { verdict: 'same' });
  assert.throws(() => order.config(config(2, Codec.Av1), 0), OrderError);
  assert.throws(() => order.config(config(2, Codec.H264, 1920, 1080), 0), OrderError);
  assert.deepEqual(order.config(config(1), 0), { verdict: 'stale' });
  assert.equal(order.screen?.epoch, 2);
});

test('media before a configuration or of an unconfigured epoch breaks the stream; older epochs are discarded', () => {
  const fresh = new StreamOrder();
  assert.throws(() => fresh.tiles(tiles(1, 1, full)), OrderError);
  assert.throws(() => fresh.video(video(1, 1, true), 0), OrderError);
  assert.throws(() => fresh.cursor(cursor(1)), OrderError);
  const order = started(5);
  assert.throws(() => order.tiles(tiles(6, 1, full)), OrderError);
  assert.equal(order.tiles(tiles(4, 1, full)), 'stale');
  assert.deepEqual(order.video(video(4, 2, true), 0), { kind: 'drop', keyframeRequest: null });
  assert.equal(order.cursor(cursor(4)), 'stale');
  assert.equal(order.cursor(cursor(5)), 'apply');
});

test('rectangles must lie inside the screen and video needs a codec', () => {
  const order = started();
  assert.throws(() => order.tiles(tiles(1, 1, { x: W - 63, y: 0, w: 64, h: 64 })), OrderError);
  assert.throws(() => order.video(video(1, 1, true, { x: 0, y: H - 1, w: 64, h: 2 }), 0), OrderError);
  assert.equal(order.tiles(tiles(1, 1, { x: W - 64, y: H - 32, w: 64, h: 32 })), 'apply');
  const tilesOnly = new StreamOrder();
  tilesOnly.config(config(1, Codec.None), 0);
  assert.throws(() => tilesOnly.video(video(1, 1, true), 0), OrderError);
});

test('revisions are non-decreasing: a full refresh may span messages of one revision, older ones are stale', () => {
  const order = started();
  assert.equal(order.tiles(tiles(1, 10, { x: 0, y: 0, w: 64, h: 64 })), 'apply');
  assert.equal(order.tiles(tiles(1, 10, { x: 64, y: 0, w: 64, h: 64 })), 'apply');
  assert.equal(order.tiles(tiles(1, 9, { x: 0, y: 0, w: 64, h: 64 })), 'stale');
  assert.equal(order.video(video(1, 9, true), 0).kind, 'drop');
  assert.equal(order.video(video(1, 11, true), 0).kind, 'decode');
});

test('tiles to video: inter frames wait for the keyframe that enters video mode', () => {
  const order = started();
  assert.equal(order.tiles(tiles(1, 1, full)), 'apply');
  // The keyframe request of the configuration is outstanding, so a stray inter frame does not repeat it at once.
  assert.deepEqual(order.video(video(1, 2, false), 10), { kind: 'drop', keyframeRequest: null });
  assert.deepEqual(order.video(video(1, 3, false), KEYFRAME_RETRY_MS), { kind: 'drop', keyframeRequest: 1 });
  const key = order.video(video(1, 4, true), KEYFRAME_RETRY_MS);
  assert.deepEqual(key, { kind: 'decode', seq: 0, keyframe: true });
  assert.deepEqual(order.video(video(1, 5, false), KEYFRAME_RETRY_MS), { kind: 'decode', seq: 1, keyframe: false });
  assert.deepEqual(order.videoOutput(0), { rect: full, parts: [full] });
  assert.deepEqual(order.videoOutput(1), { rect: full, parts: [full] });
  assert.equal(order.videoOutput(1), null);
});

test('video to tiles: the final lossless refresh wins over late video frames', () => {
  const order = started();
  const a = queue(order, video(1, 1, true));
  const b = queue(order, video(1, 2, false));
  // Leaving video mode: a full refresh at a revision after the last frame, before either frame came out.
  assert.equal(order.tiles(tiles(1, 3, { x: 0, y: 0, w: W, h: 400 })), 'apply');
  assert.equal(order.tiles(tiles(1, 3, { x: 0, y: 400, w: W, h: 400 })), 'apply');
  assert.deepEqual(order.videoOutput(a), { rect: full, parts: [] });
  assert.deepEqual(order.videoOutput(b), { rect: full, parts: [] });
});

test('a late frame draws only where no newer tile was drawn', () => {
  const order = started();
  const placement: Rect = { x: 0, y: 0, w: 640, h: 400 };
  const a = queue(order, video(1, 1, true, placement));
  const text: Rect = { x: 64, y: 64, w: 64, h: 64 };
  order.tiles(tiles(1, 2, text));
  const b = queue(order, video(1, 3, false, placement));
  const late = order.videoOutput(a);
  assert(late !== null);
  assert.equal(area(late.parts), 640 * 400 - 64 * 64);
  assert(late.parts.every((p) => subtract(p, text).length === 1 && subtract(p, text)[0] === p));
  // The next frame is newer than the tile and covers it again.
  assert.deepEqual(order.videoOutput(b), { rect: placement, parts: [placement] });
});

test('tiles drawn while no frame is queued never mask later frames', () => {
  const order = started();
  const a = queue(order, video(1, 1, true));
  assert.deepEqual(order.videoOutput(a)?.parts, [full]);
  order.tiles(tiles(1, 2, { x: 0, y: 0, w: 64, h: 64 }));
  const b = queue(order, video(1, 3, false));
  assert.deepEqual(order.videoOutput(b)?.parts, [full]);
});

test('remembered tiles are kept only while an older frame is still queued', () => {
  const order = started();
  const a = queue(order, video(1, 1, true));
  order.tiles(tiles(1, 2, { x: 0, y: 0, w: 64, h: 64 }));
  const b = queue(order, video(1, 3, false));
  order.tiles(tiles(1, 4, { x: 64, y: 0, w: 64, h: 64 }));
  assert.equal(area(order.videoOutput(a)!.parts), W * H - 2 * 64 * 64);
  // Only the tile newer than b still masks it.
  assert.equal(area(order.videoOutput(b)!.parts), W * H - 64 * 64);
});

test('too many remembered tiles skip the queued frames instead of growing', () => {
  const order = started();
  const a = queue(order, video(1, 1, true));
  const rect: Rect = { x: 0, y: 0, w: 1, h: 1 };
  for (let i = 0; i < MAX_REMEMBERED_TILES; i += 4096) {
    order.tiles(tiles(1, 2, ...Array.from({ length: 4096 }, () => rect)));
  }
  order.tiles(tiles(1, 2, rect));
  assert.equal(order.videoOutput(a), null);
  // The decoder keeps its state, so the next inter frame still decodes and draws.
  const b = queue(order, video(1, 3, false));
  assert.deepEqual(order.videoOutput(b)?.parts, [full]);
});

test('a decoder error drops the queue and asks for a keyframe, rate-limited until it arrives', () => {
  const order = started();
  const a = queue(order, video(1, 1, true), 0);
  queue(order, video(1, 2, false), 0);
  assert.equal(order.decoderLost(5000), 1);
  assert.equal(order.videoOutput(a), null);
  assert.deepEqual(order.video(video(1, 3, false), 5000 + KEYFRAME_RETRY_MS - 1), { kind: 'drop', keyframeRequest: null });
  assert.deepEqual(order.video(video(1, 4, false), 5000 + KEYFRAME_RETRY_MS), { kind: 'drop', keyframeRequest: 1 });
  assert.equal(order.keyframeRequest(5000 + KEYFRAME_RETRY_MS + 1), null);
  assert.equal(queue(order, video(1, 5, true), 7000), 2);
  assert.equal(order.keyframeRequest(99_999), null);
});

test('an inter frame that moves the video region is undecodable and asks for a keyframe', () => {
  const order = started();
  const first = queue(order, video(1, 1, true, { x: 0, y: 0, w: 640, h: 400 }));
  const moved = order.video(video(1, 2, false, { x: 640, y: 0, w: 640, h: 400 }), 3000);
  assert.deepEqual(moved, { kind: 'drop', keyframeRequest: 1 });
  assert.equal(order.videoOutput(first), null);
  // A keyframe may place the region anywhere.
  assert.equal(order.video(video(1, 3, true, { x: 640, y: 0, w: 640, h: 400 }), 3000).kind, 'decode');
});

test('an epoch change mid-stream discards the old epoch, its queued frames, and its placement', () => {
  const order = started(1);
  const a = queue(order, video(1, 100, true));
  queue(order, video(1, 101, false));
  assert.deepEqual(order.config(config(2, Codec.Av1, 1920, 1080), 500), { verdict: 'reset', keyframeRequest: 2 });
  assert.equal(order.screen?.codec, Codec.Av1);
  assert.equal(order.videoOutput(a), null);
  // Late messages of epoch 1 are dropped; epoch 2 restarts revisions and needs its own keyframe.
  assert.deepEqual(order.video(video(1, 102, false), 600), { kind: 'drop', keyframeRequest: null });
  assert.equal(order.tiles(tiles(1, 103, full)), 'stale');
  assert.equal(order.tiles(tiles(2, 1, { x: 0, y: 0, w: 1920, h: 1080 })), 'apply');
  assert.deepEqual(order.video(video(2, 2, false), 600), { kind: 'drop', keyframeRequest: null });
  const b = queue(order, video(2, 3, true, { x: 0, y: 0, w: 1920, h: 1080 }), 600);
  assert.deepEqual(order.videoOutput(b)?.parts, [{ x: 0, y: 0, w: 1920, h: 1080 }]);
});

test('scrolling: video, a lossless pause, and video again', () => {
  const order = started();
  const first = queue(order, video(1, 1, true));
  assert.deepEqual(order.videoOutput(first)?.parts, [full]);
  const second = queue(order, video(1, 2, false));
  // Motion stops: the final refresh arrives before the last frame decodes, then a static page sends nothing.
  order.tiles(tiles(1, 3, full));
  assert.deepEqual(order.videoOutput(second)?.parts, []);
  // Motion resumes with a new keyframe at a later revision, which draws over the refresh.
  const third = queue(order, video(1, 4, true));
  assert.deepEqual(order.videoOutput(third)?.parts, [full]);
});
