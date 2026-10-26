import assert from 'node:assert/strict';
import { test } from 'node:test';
import { MAX_RESIDENCE_MS, STUCK_MS, VideoStream } from '../src/decoder.ts';
import { Codec } from '../src/protocol.ts';

// A stand-in for WebCodecs' VideoDecoder whose frames come out only when the test says.

class FakeFrame {
  closed = false;
  constructor(readonly timestamp: number) {}
  close(): void {
    this.closed = true;
  }
}

const decoders: FakeDecoder[] = [];

class FakeDecoder {
  state = 'unconfigured';
  readonly queue: number[] = [];
  constructor(readonly init: { output: (frame: FakeFrame) => void; error: (e: Error) => void }) {
    decoders.push(this);
  }
  configure(): void {
    this.state = 'configured';
  }
  decode(chunk: { timestamp: number }): void {
    this.queue.push(chunk.timestamp);
  }
  /** Outputs the oldest chunk's frame. */
  emit(): void {
    this.init.output(new FakeFrame(this.queue.shift()!));
  }
  close(): void {
    this.state = 'closed';
  }
}

Object.assign(globalThis, {
  VideoDecoder: FakeDecoder,
  EncodedVideoChunk: class {
    readonly timestamp: number;
    constructor(init: { timestamp: number }) {
      this.timestamp = init.timestamp;
    }
  },
});

/** A stream on the fake decoder and a clock the test moves, with the frames it was handed. */
function stream(): { video: VideoStream; at: (ms: number) => void; out: number[]; decoder: () => FakeDecoder } {
  let clock = 1_000;
  performance.now = () => clock;
  const out: number[] = [];
  const video = new VideoStream(Codec.Av1, {
    output: (frame, seq) => {
      out.push(seq);
      frame.close();
    },
    error: () => {},
  });
  return { video, at: (ms) => (clock = ms), out, decoder: () => decoders.at(-1)! };
}

test('a burst the decoder clears within a second is no reason to reset it', () => {
  const { video, at, out, decoder } = stream();
  at(1_000);
  video.decode(new Uint8Array(1), true, 0);
  for (let seq = 1; seq < 100; seq++) video.decode(new Uint8Array(1), false, seq);
  // A frame out every 9 ms: the oldest waiting never waits a second.
  for (let i = 0; i < 100; i++) {
    at(1_000 + 9 * (i + 1));
    assert.equal(video.stuck(performance.now()), false, `after ${i} frames`);
    decoder().emit();
  }
  assert.equal(out.length, 100);
  assert.equal(video.stuck(performance.now()), false, 'nothing waits');
});

test('a decoder with nothing out of it for a second is stuck', () => {
  const { video, at } = stream();
  at(5_000);
  video.decode(new Uint8Array(1), true, 0);
  at(5_000 + STUCK_MS);
  assert.equal(video.stuck(5_000 + STUCK_MS), false);
  assert.equal(video.stuck(5_001 + STUCK_MS), true);
});

test('a decoder that keeps outputting but falls ever further behind is stuck', () => {
  const { video, at, decoder } = stream();
  // A chunk every 16 ms, a frame out every 200 ms: output never stops, the backlog grows.
  let t = 10_000;
  let seq = 0;
  at(t);
  video.decode(new Uint8Array(1), true, seq++);
  while (!video.stuck(t)) {
    t += 16;
    at(t);
    video.decode(new Uint8Array(1), false, seq++);
    if (seq % 12 === 0) decoder().emit();
    assert(t < 10_000 + MAX_RESIDENCE_MS + 400, 'never found stuck');
  }
  assert(t > 10_000 + MAX_RESIDENCE_MS, `stuck after ${t - 10_000} ms`);
});

test('a long idle before a chunk is no stall, and a reset forgets what waited', () => {
  const { video, at, out, decoder } = stream();
  at(20_000);
  video.decode(new Uint8Array(1), true, 0);
  decoder().emit();
  // Nothing for a minute, then a chunk.
  at(80_000);
  video.decode(new Uint8Array(1), false, 1);
  assert.equal(video.stuck(80_000 + STUCK_MS - 1), false);
  video.reset();
  assert.equal(video.stuck(80_000 + MAX_RESIDENCE_MS + 1), false, 'nothing waits after a reset');
  // A frame of the reset decoder that still comes out is not handed on.
  const retired = decoder();
  retired.emit();
  assert.equal(out.length, 1);
  video.decode(new Uint8Array(1), true, 2);
  assert(decoder() !== retired, 'a keyframe opened a new decoder');
});
