import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Sound, type SoundState } from '../src/audio.ts';
import type { MessageOf } from '../src/protocol.ts';

// Minimal stand-ins for the Web Audio and WebCodecs objects Sound uses, so its scheduling runs under Node.

class FakeSource {
  started: number | null = null;
  stopped = false;
  buffer: FakeBuffer | null = null;
  onended: (() => void) | null = null;
  connect(): void {}
  start(at: number): void {
    this.started = at;
  }
  stop(): void {
    this.stopped = true;
  }
}

class FakeGain {
  targets: number[] = [];
  gain = { setTargetAtTime: (value: number) => this.targets.push(value) };
  connect(): void {}
}

class FakeBuffer {
  readonly duration: number;
  readonly channels: Float32Array[];
  constructor(channels: number, frames: number, rate: number) {
    this.duration = frames / rate;
    this.channels = Array.from({ length: channels }, () => new Float32Array(frames));
  }
  getChannelData(channel: number): Float32Array {
    return this.channels[channel]!;
  }
}

class FakeContext {
  static last: FakeContext;
  state: AudioContextState = 'running';
  currentTime = 10;
  destination = {};
  sources: FakeSource[] = [];
  onstatechange: (() => void) | null = null;
  constructor() {
    FakeContext.last = this;
  }
  createBuffer(channels: number, frames: number, rate: number): FakeBuffer {
    return new FakeBuffer(channels, frames, rate);
  }
  gain = new FakeGain();
  createGain(): FakeGain {
    return this.gain;
  }
  createBufferSource(): FakeSource {
    const source = new FakeSource();
    this.sources.push(source);
    return source;
  }
  resume(): Promise<void> {
    return Promise.resolve();
  }
  close(): Promise<void> {
    return Promise.resolve();
  }
  setState(state: AudioContextState): void {
    this.state = state;
    this.onstatechange?.();
  }
}

class FakeDecoder {
  static all: FakeDecoder[] = [];
  state = 'unconfigured';
  decodeQueueSize = 0;
  chunks: { timestamp: number }[] = [];
  constructor(readonly init: { output: (data: unknown) => void; error: () => void }) {
    FakeDecoder.all.push(this);
  }
  static isConfigSupported(): Promise<{ supported: boolean }> {
    return Promise.resolve({ supported: true });
  }
  configure(): void {
    this.state = 'configured';
  }
  decode(chunk: { timestamp: number }): void {
    this.chunks.push(chunk);
  }
  close(): void {
    this.state = 'closed';
  }
  /** Delivers the decoded output of the chunk stamped `timestamp`. */
  output(timestamp: number): void {
    this.init.output({
      timestamp,
      numberOfChannels: 2,
      numberOfFrames: 960,
      sampleRate: 48_000,
      // Each plane's samples are its number plus one, so a test sees which plane landed where.
      copyTo(destination: Float32Array, { planeIndex }: { planeIndex: number }): void {
        destination.fill(planeIndex + 1);
      },
      close(): void {},
    });
  }
}

Object.assign(globalThis, {
  AudioContext: FakeContext,
  AudioDecoder: FakeDecoder,
  EncodedAudioChunk: class {
    readonly timestamp: number;
    constructor(init: { timestamp: number }) {
      this.timestamp = init.timestamp;
    }
  },
  document: { addEventListener(): void {}, removeEventListener(): void {} },
});

const packet = (timestampUs: number): MessageOf<'audio'> => ({
  type: 'audio',
  timestampUs: BigInt(timestampUs),
  payload: Uint8Array.of(0xfc),
});

async function started(): Promise<{ sound: Sound; context: FakeContext; states: SoundState[] }> {
  FakeDecoder.all = [];
  const states: SoundState[] = [];
  const sound = new Sound((s) => states.push(s));
  await Promise.resolve();
  await Promise.resolve();
  return { sound, context: FakeContext.last, states };
}

// The client's clock equals the server's: offset 0, and "now" is passed in microseconds.
const now = (): number => Math.round(performance.now() * 1000);

test('consecutive packets play back to back, a little ahead of the clock', async () => {
  const { sound, context, states } = await started();
  assert.equal(states.at(-1), 'playing');
  const t = now();
  sound.push(packet(t), t, 0);
  sound.push(packet(t + 20_000), t, 0);
  const decoder = FakeDecoder.all[0]!;
  decoder.output(t);
  decoder.output(t + 20_000);
  assert.deepEqual(
    context.sources.map((s) => s.started),
    [10.06, 10.08],
  );
});

test('each channel of the decoded sound plays from its own plane', async () => {
  const { sound, context } = await started();
  const t = now();
  sound.push(packet(t), t, 0);
  FakeDecoder.all[0]!.output(t);
  const channels = context.sources[0]!.buffer!.channels;
  assert.deepEqual(
    channels.map((samples) => [samples.length, samples[0], samples.at(-1)]),
    [
      [960, 1, 1],
      [960, 2, 2],
    ],
  );
});

test('sound that waited behind a stall is dropped when it would play, not only on arrival', async () => {
  const { sound, context } = await started();
  const t = now();
  sound.push(packet(t), t, 0);
  // Decoded only after the packet became older than the bound (as after a long main-thread stall).
  const realNow = performance.now;
  performance.now = () => realNow.call(performance) + 1_000;
  try {
    FakeDecoder.all[0]!.output(t);
  } finally {
    performance.now = realNow;
  }
  assert.equal(context.sources.length, 0);
});

test('sound after a pause plays, though the decoder stamps its output on the timeline from before', async () => {
  const { sound, context } = await started();
  const realNow = performance.now;
  let clock = realNow.call(performance);
  performance.now = () => clock;
  try {
    const t = now();
    sound.push(packet(t), t, 0);
    const decoder = FakeDecoder.all[0]!;
    decoder.output(t);
    // Muted in the page for 5 s, twice: the decoder, kept, goes on stamping right after its last output.
    for (let i = 0; i < 2; i++) {
      clock += 5_000;
      const at = now();
      sound.push(packet(at), at, 0);
      sound.push(packet(at + 20_000), at, 0);
      decoder.output(t + (2 * i + 1) * 20_000);
      decoder.output(t + (2 * i + 2) * 20_000);
    }
  } finally {
    performance.now = realNow;
  }
  assert.equal(FakeDecoder.all.length, 1);
  assert.equal(context.sources.length, 5, 'every packet after the pauses plays');
});

test('the decoder holds a bounded number of packets, counted until their output', async () => {
  const { sound } = await started();
  const t = now();
  // The decoder reports an empty queue, but none of its output has come out yet.
  for (let i = 0; i < 15; i++) sound.push(packet(t + i * 20_000), t, 0);
  assert.equal(FakeDecoder.all[0]!.chunks.length, 10);
});

test('a suspended context drops what was scheduled and the next packet starts a new stream', async () => {
  const { sound, context, states } = await started();
  const t = now();
  sound.push(packet(t), t, 0);
  FakeDecoder.all[0]!.output(t);
  context.setState('suspended');
  assert.equal(context.sources[0]!.stopped, true);
  assert.equal(FakeDecoder.all[0]!.state, 'closed');
  assert.equal(states.at(-1), 'blocked');
  sound.push(packet(t + 20_000), t, 0);
  assert.equal(FakeDecoder.all.length, 1, 'nothing is decoded while suspended');
  context.setState('running');
  sound.push(packet(t + 40_000), t, 0);
  assert.equal(FakeDecoder.all.length, 2, 'a fresh decoder after resuming');
});

test('a gap in the timestamps keeps what is scheduled and the decoder, and plays on after it', async () => {
  const { sound, context } = await started();
  const t = now();
  sound.push(packet(t), t, 0);
  sound.push(packet(t + 20_000), t, 0);
  // A packet lost: the next one arrives while the first two still wait to be decoded.
  sound.push(packet(t + 60_000), t, 0);
  assert.equal(FakeDecoder.all.length, 1, 'one decoder throughout');
  const decoder = FakeDecoder.all[0]!;
  assert.equal(decoder.state, 'configured');
  decoder.output(t);
  decoder.output(t + 20_000);
  decoder.output(t + 60_000);
  assert.deepEqual(
    context.sources.map((s) => [s.started, s.stopped]),
    [
      [10.06, false],
      [10.08, false],
      [10.1, false],
    ],
  );
});

test('sound never queues more than 300 ms ahead', async () => {
  const { sound, context } = await started();
  const t = now();
  // 30 consecutive packets decoded while the audio clock stands still.
  for (let i = 0; i < 30; i++) {
    sound.push(packet(t + i * 20_000), t, 0);
    FakeDecoder.all[0]!.output(t + i * 20_000);
  }
  assert(context.sources.length < 30, 'some were dropped');
  const ahead = Math.max(...context.sources.map((s) => s.started ?? 0)) - context.currentTime;
  assert(ahead <= 0.3, `${ahead} s scheduled ahead`);
});

test('a new stream after the scheduled sound ran out starts a little ahead of the clock', async () => {
  const { sound, context } = await started();
  const t = now();
  sound.push(packet(t), t, 0);
  FakeDecoder.all[0]!.output(t);
  context.currentTime = 11;
  sound.push(packet(t + 500_000), t + 500_000, 0);
  FakeDecoder.all[0]!.output(t + 500_000);
  assert.equal(context.sources.at(-1)!.started, 11.06);
});

test('output from a decoder closed by muting is not played after unmuting', async () => {
  const { sound, context } = await started();
  const t = now();
  sound.push(packet(t), t, 0);
  sound.setMuted(true);
  sound.setMuted(false);
  sound.push(packet(t + 20_000), t, 0);
  assert.equal(FakeDecoder.all.length, 2);
  FakeDecoder.all[0]!.output(t);
  assert.equal(context.sources.length, 0, 'the closed decoder delivered nothing playable');
  FakeDecoder.all[1]!.output(t + 20_000);
  // Muting dropped the old schedule: the new stream starts a little ahead of the clock, not after it.
  assert.deepEqual(
    context.sources.map((s) => s.started),
    [10.06],
  );
});

test('muting stops what plays and ignores packets until unmuted', async () => {
  const { sound, context, states } = await started();
  const t = now();
  sound.push(packet(t), t, 0);
  FakeDecoder.all[0]!.output(t);
  sound.setMuted(true);
  assert.equal(states.at(-1), 'muted');
  assert.equal(context.sources[0]!.stopped, true);
  sound.push(packet(t + 20_000), t, 0);
  assert.equal(FakeDecoder.all.length, 1);
});

test('the volume ramps to a level between silent and full, and ignores nonsense', async () => {
  const { sound, context } = await started();
  sound.setVolume(0.4);
  sound.setVolume(7);
  sound.setVolume(-1);
  sound.setVolume(Number.NaN);
  sound.setVolume(Number.POSITIVE_INFINITY);
  assert.deepEqual(context.gain.targets, [0.4, 1, 0]);
});

test('a closed sound says nothing more, though its context closes and its support is answered late', async () => {
  const states: SoundState[] = [];
  const sound = new Sound((s) => states.push(s));
  const context = FakeContext.last;
  sound.close();
  const said = states.length;
  context.setState('closed');
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(states.length, said);
});

test('a packet that comes twice, or after a later one, is dropped before the decoder', async () => {
  const { sound } = await started();
  const t = 50_000_000;
  sound.push(packet(t), t, 0);
  sound.push(packet(t + 20_000), t, 0);
  sound.push(packet(t + 20_000), t, 0);
  sound.push(packet(t), t, 0);
  sound.push(packet(t + 40_000), t, 0);
  assert.deepEqual(
    FakeDecoder.all.at(-1)!.chunks.map((c) => c.timestamp),
    [t, t + 20_000, t + 40_000],
  );
});
