import assert from 'node:assert/strict';
import { test } from 'node:test';
import { clock, Kind, parseRecords, Playback, pointerOf, type ReplayIndex, type ReplayRecord, seekFrom, segmentFor, unrecorded } from '../src/ui/replay.ts';

/** Records as the admin writes them (admin/src/replay.rs, `Builder::push`). */
function records(...list: [number, number, number[]][]): Uint8Array {
  const out: number[] = [];
  for (const [t, kind, bytes] of list) {
    out.push((t >>> 24) & 255, (t >>> 16) & 255, (t >>> 8) & 255, t & 255, kind);
    const n = bytes.length;
    out.push((n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255, ...bytes);
  }
  return new Uint8Array(out);
}

test('records read as the admin writes them, a record cut short ending them', () => {
  const data = records([7, Kind.Pointer, [0, 1, 0, 2]], [9, Kind.Message, [109]]);
  const read = parseRecords(data);
  assert.deepEqual(read.map((r) => [r.t, r.kind, [...r.bytes]]), [[7, Kind.Pointer, [0, 1, 0, 2]], [9, Kind.Message, [109]]]);
  assert.equal(parseRecords(data.subarray(0, data.length - 1)).length, 1);
  assert.deepEqual(pointerOf(new Uint8Array([0, 1, 0, 2])), { x: 1, y: 2 });
  assert.equal(pointerOf(new Uint8Array([255, 255, 255, 255])), null);
});

const index: ReplayIndex = {
  started: 1_700_000_000_000,
  segments: [
    { n: 0, start: 0, end: 10_000, size: 1, starts: 'watch' },
    { n: 1, start: 10_000, end: 20_000, size: 1, starts: 'refresh' },
    { n: 2, start: 60_000, end: 70_000, size: 1, starts: 'watch' },
  ],
  stopped: null,
  recording: false,
};

test('a moment starts from the last segment before it; what was not recorded and the clock read plainly', () => {
  assert.equal(segmentFor(index, 0), 0);
  assert.equal(segmentFor(index, 15_000), 1);
  assert.equal(segmentFor(index, 45_000), 1);
  assert.equal(segmentFor(index, 65_000), 2);
  assert.deepEqual(unrecorded(index), [[20_000, 60_000]]);
  assert.equal(clock(65_000), '1:05');
  assert.equal(clock(3_725_000), '1:02:05');
});

/** A screen that notes what it was fed. */
function sink() {
  const fed: string[] = [];
  return { fed, reset: () => fed.push('reset'), message: (b: Uint8Array) => fed.push(`m${b[0]}`), pointer: (x: number, y: number) => fed.push(`p${x},${y}`) };
}
const segments: Record<number, ReplayRecord[]> = {
  0: parseRecords(records([0, Kind.Message, [1]], [500, Kind.Pointer, [0, 5, 0, 6]], [4000, Kind.Message, [2]])),
  1: parseRecords(records([10_000, Kind.Message, [3]], [12_000, Kind.Message, [4]])),
  2: parseRecords(records([60_000, Kind.Message, [5]])),
};
const load = async (n: number) => segments[n]!;
const settle = () => new Promise((r) => setTimeout(r, 0));

test('a seek before the first picture shows it, without moving the clock', async () => {
  const screen = sink();
  const late = parseRecords(records([0, Kind.Message, [2]], [800, Kind.Message, [3]], [900, Kind.Message, [3]]));
  const playback = new Playback({ started: 0, stopped: null, recording: false, segments: [{ n: 0, start: 0, end: 900, size: 1 }] }, async () => late, screen);
  await playback.seek(0);
  assert.deepEqual(screen.fed, ['reset', 'm2', 'm3']);
  assert.equal(playback.time, 0);
});

test('only the segment a seek opened has its picture fed early', async () => {
  const screen = sink();
  const opened = parseRecords(records([100, Kind.Message, [2]], [800, Kind.Message, [3]]));
  const next = parseRecords(records([5_000, Kind.Message, [2]], [9_000, Kind.Message, [3]]));
  const list = [
    { n: 0, start: 0, end: 5_000, size: 1 },
    { n: 1, start: 5_000, end: 10_000, size: 1 },
  ];
  const playback = new Playback({ started: 0, stopped: null, recording: false, segments: list }, async (n) => (n === 0 ? opened : next), screen);
  await playback.seek(0);
  assert.deepEqual(screen.fed, ['reset', 'm2', 'm3']);
  await playback.seek(0);
  playback.playing = true;
  playback.skipIdle = false;
  playback.advance(5_500);
  await settle();
  assert.deepEqual(screen.fed.slice(-1), ['m2'], 'what the next segment holds for later waits for its time');
});

test('what was deleted for room shows as not recorded, and playback goes on from what is left', async () => {
  const screen = sink();
  const list = [
    { n: 4, start: 40_000, end: 50_000, size: 1, starts: 'refresh' as const },
    { n: 5, start: 50_000, end: 60_000, size: 1, starts: 'refresh' as const },
  ];
  const kept = { started: 0, stopped: null, recording: false, segments: list };
  assert.deepEqual(unrecorded(kept), [[0, 40_000]]);
  const records_ = (n: number) => parseRecords(records([n * 10_000, Kind.Message, [3]]));
  const playback = new Playback(kept, async (n) => records_(n), screen);
  await playback.seek(5_000);
  assert.equal(playback.time, 40_000, 'nothing before what is left');
  // Its feeding segment stays by number when the head goes.
  playback.index = { ...kept, segments: list.slice(0) };
  await playback.seek(55_000);
  playback.index = { ...kept, segments: list.slice(1) };
  playback.playing = true;
  playback.skipIdle = false;
  playback.advance(1000);
  assert.equal(playback.time, 56_000);
  // And when its own goes, it moves to what is left.
  playback.index = { ...kept, segments: [{ n: 9, start: 90_000, end: 95_000, size: 1, starts: 'refresh' as const }] };
  await settle();
  assert.equal(playback.time, 90_000);
});

test('a head deleted while a seek loads leaves it on the segment it asked for', async () => {
  const screen = sink();
  const list = [0, 1, 2, 3].map((n) => ({ n, start: n * 10_000, end: (n + 1) * 10_000, size: 1, starts: 'refresh' as const }));
  const kept = { started: 0, stopped: null, recording: false, segments: list };
  let release = () => {};
  const held = new Promise<void>((r) => (release = r));
  const playback = new Playback(kept, async (n) => {
    if (n === 1) await held;
    return parseRecords(records([n * 10_000, Kind.Message, [n + 10]]));
  }, screen);
  const seeking = playback.seek(25_000);
  playback.index = { ...kept, segments: list.slice(1) };
  release();
  await seeking;
  playback.playing = true;
  playback.skipIdle = false;
  playback.advance(10_000);
  await settle();
  assert.deepEqual(screen.fed.slice(-2), ['m12', 'm13'], 'on from where it was, none skipped');
});

test('playing feeds what is due at its speed, skips idle time and goes on across segments', async () => {
  const screen = sink();
  const playback = new Playback(index, load, screen);
  await playback.seek(0);
  assert.deepEqual(screen.fed, ['reset', 'm1']);
  playback.playing = true;
  playback.speed = 2;
  playback.advance(300);
  assert.deepEqual(screen.fed.slice(2), ['p5,6'], 'at 600 ms');
  // 600 ms to the next record at 4 s: past the idle bound, it jumps to just before it.
  playback.advance(10);
  assert(playback.time >= 3_700 && playback.time < 4_000, `${playback.time}`);
  playback.advance(200);
  assert.deepEqual(screen.fed.slice(3), ['m2']);
  // On into the next segment without starting over, and over the unrecorded stretch to the last.
  playback.advance(4_000);
  await settle();
  playback.advance(2_000);
  await settle();
  assert.deepEqual(screen.fed.slice(4), ['m3', 'm4']);
  // Past the last record of its segment, the jump lands just before the next segment, then reaches it.
  playback.advance(1);
  await settle();
  playback.advance(400);
  await settle();
  assert.equal(screen.fed.at(-1), 'm5');
  assert(!screen.fed.slice(1).includes('reset'), 'never started over while playing');
});

test('a seek starts the screen over from the segment before, when it follows on, and an older seek feeds nothing', async () => {
  const screen = sink();
  let release: () => void = () => {};
  let held = false;
  // The last segment loads slowly the first time it is asked for.
  const slow = (n: number) => {
    if (n !== 2 || held) return load(n);
    held = true;
    return new Promise<ReplayRecord[]>((r) => (release = () => r(segments[2]!)));
  };
  const playback = new Playback(index, slow, screen);
  const older = playback.seek(65_000);
  // 15 s is in the second segment, which follows the first at once: it starts from the first, for a whole picture.
  await playback.seek(15_000);
  release();
  await older;
  assert.deepEqual(screen.fed, ['reset', 'm1', 'p5,6', 'm2', 'm3', 'm4']);
  // 65 s is in the last segment, after an unrecorded stretch: it starts from that one.
  await playback.seek(65_000);
  assert.deepEqual(screen.fed.slice(6), ['reset', 'm5']);
});

test('a gap starts the screen over, and a segment that does not load stops the replay and says so', async () => {
  const screen = sink();
  const gapped = { 0: parseRecords(records([0, Kind.Message, [1]], [100, Kind.Gap, []], [200, Kind.Message, [2]])) } as Record<number, ReplayRecord[]>;
  const errors: unknown[] = [];
  const one = { ...index, segments: [{ n: 0, start: 0, end: 1_000, size: 1 }, { n: 1, start: 1_000, end: 2_000, size: 1 }] };
  const playback = new Playback(one, async (n) => (n === 0 ? gapped[0]! : Promise.reject(new Error('gone'))), screen, (e) => errors.push(e));
  await playback.seek(500);
  assert.deepEqual(screen.fed, ['reset', 'm1', 'reset', 'm2']);
  playback.playing = true;
  playback.skipIdle = false;
  playback.advance(600);
  await settle();
  assert.equal(errors.length, 1);
  assert.equal(playback.playing, false);
});

test('a seek goes back to the first segment of its watch, or to the second refresh before the moment', () => {
  const at = (starts: ('watch' | 'refresh' | 'more')[], gapAfter = -1) => ({
    ...index,
    segments: starts.map((s, i) => ({ n: i, start: i * 10_000 + (i > gapAfter && gapAfter >= 0 ? 5_000 : 0), end: (i + 1) * 10_000 + (i > gapAfter && gapAfter >= 0 ? 5_000 : 0), size: 1, starts: s })),
  });
  assert.equal(seekFrom(at(['watch', 'refresh', 'refresh', 'refresh']), 35_000), 2, 'two refreshes back');
  assert.equal(seekFrom(at(['watch', 'refresh', 'refresh', 'more', 'more']), 45_000), 1, 'cuts for size go on from before');
  assert.equal(seekFrom(at(['watch', 'refresh', 'more', 'more']), 35_000), 0, 'one refresh only: the watch’s first');
  assert.equal(seekFrom(at(['watch', 'more', 'refresh']), 25_000), 0, 'or the watch’s first');
  assert.equal(seekFrom(at(['watch', 'refresh', 'refresh'], 0), 25_000), 1, 'never across a stretch not recorded');
});
