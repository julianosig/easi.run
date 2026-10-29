import assert from 'node:assert/strict';
import { test } from 'node:test';
import vectors from '../../protocol/vectors/v4.txt';
import {
  CODEC_AV1,
  CODEC_H264,
  type ClientMessage,
  Codec,
  decode,
  encode,
  type ErrorKind,
  KeyKind,
  LeaseHolder,
  MAX_AUDIO_PACKET,
  MAX_COPIED,
  MAX_FILE_NAME,
  MAX_CURSOR_IMAGE,
  MAX_SERVER_MESSAGE,
  MAX_TILES,
  type Message,
  type ServerMessage,
  PointerKind,
  ProtocolError,
  RecordPhase,
  VERSION,
} from '../src/protocol.ts';

const bytes = (...b: number[]): Uint8Array => Uint8Array.from(b);
const rect = { x: 64, y: 128, w: 640, h: 400 };

const LAYOUT = { x: 7_400, y: 6_800, side: 2_400, radius: 100 };

// The values protocol/tests/vectors.rs encodes into vectors/v4.txt: what a client sends, as protocol/src/lib.rs's
// `Message::direction` has them, and what a server sends.
const clientSamples: Record<string, ClientMessage> = {
  hello: { type: 'hello', version: VERSION, codecs: CODEC_AV1 | CODEC_H264 },
  'keyframe-request': { type: 'keyframe-request', epoch: 7 },
  'media-ack': { type: 'media-ack', revision: 1002n },
  'udp-request': { type: 'udp-request' },
  'client-stats': { type: 'client-stats', presented: 59, decodeUsP50: 1_500, hardware: true },
  'time-ping': { type: 'time-ping', clientTimeUs: 42n },
  pointer: { type: 'pointer', leaseEpoch: 3, kind: PointerKind.Down, button: 0, x: 10, y: 20 },
  'pointer-move': { type: 'pointer', leaseEpoch: 3, kind: PointerKind.Move, button: 0, x: 11, y: 21 },
  'pointer-up': { type: 'pointer', leaseEpoch: 3, kind: PointerKind.Up, button: 2, x: 12, y: 22 },
  wheel: { type: 'wheel', leaseEpoch: 3, x: 10, y: 20, dx: 0, dy: -120 },
  key: { type: 'key', leaseEpoch: 3, kind: KeyKind.Down, code: 30, text: '' },
  'key-up': { type: 'key', leaseEpoch: 3, kind: KeyKind.Up, code: 30, text: '' },
  'key-text': { type: 'key', leaseEpoch: 3, kind: KeyKind.Text, code: 0, text: 'ção' },
  clipboard: { type: 'clipboard', leaseEpoch: 3, text: 'hello' },
  'upload-start': { type: 'upload-start', leaseEpoch: 3, upload: 4, size: 70_000n, name: 'foto.png' },
  'upload-data': { type: 'upload-data', leaseEpoch: 3, upload: 4, offset: 61_440n, data: bytes(9, 8, 7) },
  'file-choose': { type: 'file-choose', leaseEpoch: 3, request: 9, uploads: [4, 5] },
  'file-drop': { type: 'file-drop', leaseEpoch: 3, x: 640, y: 400, uploads: [4] },
  'file-drag': { type: 'file-drag', leaseEpoch: 3, x: 640, y: 400, types: ['image/png', ''] },
  'file-drag-end': { type: 'file-drag-end', leaseEpoch: 3 },
  'record-start': {
    type: 'record-start',
    leaseEpoch: 3,
    recording: 2,
    layout: LAYOUT,
    camera: true,
    microphone: true,
    pageSound: false,
  },
  'record-camera': { type: 'record-camera', leaseEpoch: 3, recording: 2, timestampUs: 123_456_789n, jpeg: bytes(0xff, 0xd8, 0xff) },
  'record-microphone': {
    type: 'record-microphone',
    leaseEpoch: 3,
    recording: 2,
    timestampUs: 123_476_789n,
    payload: bytes(0xfc, 0xff, 0xfe),
  },
  'record-stop': { type: 'record-stop', leaseEpoch: 3, recording: 2 },
  'record-layout': { type: 'record-layout', leaseEpoch: 3, recording: 2, layout: LAYOUT },
};
const serverSamples: Record<string, ServerMessage> = {
  config: {
    type: 'config',
    version: VERSION,
    epoch: 7,
    width: 1280,
    height: 800,
    codec: Codec.H264,
    maxFps: 60,
  },
  'config-av1': { type: 'config', version: VERSION, epoch: 8, width: 1280, height: 800, codec: Codec.Av1, maxFps: 30 },
  'config-tiles': { type: 'config', version: VERSION, epoch: 9, width: 1280, height: 800, codec: Codec.None, maxFps: 60 },
  video: {
    type: 'video',
    epoch: 7,
    revision: 1001n,
    rect,
    keyframe: true,
    timestampUs: 123_456_789n,
    payload: bytes(0, 0, 0, 1, 0x67),
  },
  copied: { type: 'copied', leaseEpoch: 3, text: 'ção' },
  'file-offer': { type: 'file-offer', leaseEpoch: 3, file: 7, size: 5_000_000_000n, name: 'relatório.pdf' },
  'file-offer-unsized': { type: 'file-offer', leaseEpoch: 3, file: 8, size: null, name: 'a' },
  'file-link': { type: 'file-link', leaseEpoch: 3, file: 7, ticket: Uint8Array.from({ length: 32 }, (_, i) => i) },
  'file-end': { type: 'file-end', leaseEpoch: 3, file: 7, size: 5_000_000_000n },
  'file-gone': { type: 'file-gone', leaseEpoch: 3, file: 7 },
  'file-request': { type: 'file-request', leaseEpoch: 3, request: 9, multiple: true },
  'file-request-end': { type: 'file-request-end', leaseEpoch: 3, request: 9 },
  'upload-ack': { type: 'upload-ack', leaseEpoch: 3, upload: 4, received: 61_440n },
  'upload-done': { type: 'upload-done', leaseEpoch: 3, upload: 4, stored: true },
  audio: { type: 'audio', timestampUs: 123_476_789n, payload: bytes(0xfc, 0xff, 0xfe) },
  tiles: {
    type: 'tiles',
    epoch: 7,
    revision: 1002n,
    rects: [
      { x: 0, y: 0, w: 64, h: 64 },
      { x: 64, y: 0, w: 2, h: 1 },
    ],
    rawLen: 11,
    data: bytes(0x63, 0x54, 0x70, 0x48, 0x60, 0x60, 0x64, 0x62, 0x66, 0x61, 0x65, 0x03, 0x00),
  },
  cursor: { type: 'cursor', epoch: 7, revision: 1003n, x: 300, y: 200, shape: 1, hotX: 0, hotY: 0, image: bytes() },
  'udp-offer': {
    type: 'udp-offer',
    generation: 2,
    certSha256: Uint8Array.from({ length: 32 }, (_, i) => 0xa0 + i),
    url: 'https://203.0.113.7:9443/media?t=AAEC',
  },
  'media-moved': { type: 'media-moved', generation: 2 },
  'media-fallback': { type: 'media-fallback', generation: 2 },
  'server-stats': {
    type: 'server-stats',
    serverTimeUs: 5_000_000n,
    frames: 60,
    bytes: 250_000,
    encodeUsP50: 4_000,
    encodeUsP95: 9_000,
  },
  'time-pong': { type: 'time-pong', clientTimeUs: 42n, serverTimeUs: 99n },
  lease: { type: 'lease', leaseEpoch: 3, holder: LeaseHolder.You },
  'lease-agent': { type: 'lease', leaseEpoch: 4, holder: LeaseHolder.Agent },
  'lease-other': { type: 'lease', leaseEpoch: 5, holder: LeaseHolder.Other },
  'record-state': {
    type: 'record-state',
    leaseEpoch: 3,
    recording: 2,
    phase: RecordPhase.Recording,
    file: 9,
    seconds: 75,
    bytes: 5_000_000_000n,
  },
  'record-state-ended': {
    type: 'record-state',
    leaseEpoch: 3,
    recording: 2,
    phase: RecordPhase.Ended,
    file: 9,
    seconds: 80,
    bytes: 6n,
  },
  'record-state-failed': {
    type: 'record-state',
    leaseEpoch: 3,
    recording: 2,
    phase: RecordPhase.Failed,
    file: 0,
    seconds: 3,
    bytes: 0n,
  },
};
const samples: Record<string, Message> = { ...clientSamples, ...serverSamples };

function hex(b: Uint8Array): string {
  return Array.from(b, (v) => v.toString(16).padStart(2, '0')).join('');
}

function unhex(s: string): Uint8Array {
  assert(/^([0-9a-f]{2})*$/.test(s), `not hex: ${s}`);
  return Uint8Array.from(s.match(/../g) ?? [], (h) => parseInt(h, 16));
}

/** The download messages between the display and the admin, which never reach a browser. */
const ADMIN_ONLY = ['file-data', 'file-read', 'file-cancel'];
const golden = vectors
  .split('\n')
  .filter((line) => line !== '')
  .map((line) => {
    const [name, value, ...rest] = line.split(' ');
    assert(name !== undefined && value !== undefined && rest.length === 0, `malformed vector line: ${line}`);
    return { name, bytes: unhex(value) };
  });

/** A message from the server that the browser refuses with `kind` (and `detail`, when given). */
function refuses(b: Uint8Array, kind: ErrorKind, detail?: string): void {
  assert.throws(
    () => decode(b),
    (e: unknown) => e instanceof ProtocolError && e.kind === kind && (detail === undefined || e.detail === detail),
  );
}

test('every golden vector of a client message encodes byte for byte, and of a server message decodes to its value', () => {
  const spoken = golden.filter((v) => !ADMIN_ONLY.includes(v.name));
  assert.deepEqual(spoken.map((v) => v.name).sort(), Object.keys(samples).sort());
  for (const vector of spoken) {
    const sent = clientSamples[vector.name];
    if (sent) assert.equal(hex(encode(sent)), hex(vector.bytes), vector.name);
    else assert.deepEqual(decode(vector.bytes), serverSamples[vector.name], vector.name);
  }
});

/** A copy of the golden encoding of sample `name`. */
const sent = (name: string): Uint8Array => Uint8Array.from(golden.find((v) => v.name === name)!.bytes);

test('refuses malformed messages exactly as protocol/tests/vectors.rs', () => {
  refuses(bytes(), 'empty');
  refuses(bytes(0x7f), 'unknown-kind', '0x7f');
  // A client's message, and the download messages the admin keeps, are no server message for the browser.
  refuses(sent('hello'), 'unknown-kind', '0x01');
  for (const name of ADMIN_ONLY) refuses(sent(name), 'unknown-kind');
  // A zero-size configuration.
  refuses(bytes(0x02, VERSION, 0, 7, 0, 0, 0, 0, 0, 0x20, 3, 2, 60), 'invalid-value', 'configuration');
  // An offer of an address that is not https, or not printable.
  const offer = (url: string): Uint8Array =>
    Uint8Array.from([0x2b, 2, 0, 0, 0, ...new Array<number>(32).fill(7), ...new TextEncoder().encode(url)]);
  for (const bad of ['http://host:9443/media', 'https://host/a b', '']) refuses(offer(bad), 'invalid-value', 'udp address');
  refuses(offer(`https://${'a'.repeat(512)}`), 'too-large');
  refuses(bytes(0x2b, 2, 0, 0, 0, 7), 'truncated');
});

test('refuses out-of-range values of every closed field', () => {
  const config = sent('config');
  refuses(Uint8Array.from(config).fill(3, 11, 12), 'invalid-value', 'codec');
  refuses(Uint8Array.from(config).fill(0, 12, 13), 'invalid-value', 'configuration');
  // A tile wider than MAX_TILE_SIDE (the rectangle's width sits at bytes 19-20 of the tiles sample).
  const wide = sent('tiles');
  wide.set([1, 1], 19);
  refuses(wide, 'invalid-value', 'tile size');
  // A records length shorter than all solid records or longer than all RGB ones (it follows the two rectangles).
  for (const rawLen of [7, 64 * 64 * 3 + 1 + 7 + 1]) {
    const bad = sent('tiles');
    new DataView(bad.buffer).setUint32(31, rawLen, true);
    refuses(bad, 'invalid-value', 'tile records length');
  }
  refuses(bytes(0x0b, 3, 0, 0, 0, 3), 'invalid-value', 'lease holder');
  const video = sent('video');
  refuses(Uint8Array.from(video).fill(2, 21, 22), 'invalid-value', 'flag');
  refuses(Uint8Array.from(video).fill(0, 17, 19), 'invalid-value', 'rectangle');
  refuses(video.subarray(0, 20), 'truncated');
  // A recording state of a phase the protocol does not number.
  refuses(sent('record-state').fill(3, 9, 10), 'invalid-value', 'recording phase');
});

test('refuses tiles, cursor images, and texts over their bounds', () => {
  // A file offer without a name or with a name over its bound.
  const offer = bytes(0x0e, 3, 0, 0, 0, 7, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0);
  // A size said to be unknown must be zero.
  refuses(bytes(0x0e, 3, 0, 0, 0, 7, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0x61), 'invalid-value', 'file size');
  refuses(offer, 'invalid-value', 'file name');
  const named = new Uint8Array(offer.length + MAX_FILE_NAME + 1).fill(0x61);
  named.set(offer);
  refuses(named, 'too-large');
  // A download ticket of another length.
  for (const [length, kind] of [[31, 'truncated'], [33, 'trailing-bytes']] as const) {
    const link = new Uint8Array(9 + length);
    link.set([0x20, 3, 0, 0, 0, 7, 0, 0, 0]);
    refuses(link, kind);
  }
  // Copied text over its bound, or not UTF-8.
  const copied = new Uint8Array(5 + MAX_COPIED + 1).fill(0x61);
  copied.set([0x0d, 3, 0, 0, 0]);
  refuses(copied, 'too-large');
  refuses(bytes(0x0d, 3, 0, 0, 0, 0xff), 'invalid-value', 'utf-8');
  // An empty or over-long audio packet.
  refuses(bytes(0x0c, 0, 0, 0, 0, 0, 0, 0, 0), 'invalid-value', 'audio packet size');
  const longAudio = new Uint8Array(9 + MAX_AUDIO_PACKET + 1).fill(0xfc);
  longAudio.set([0x0c, 0, 0, 0, 0, 0, 0, 0, 0]);
  refuses(longAudio, 'invalid-value', 'audio packet size');
  const tooMany = new Uint8Array(15);
  tooMany.set([0x04, 7, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
  new DataView(tooMany.buffer).setUint16(13, MAX_TILES + 1, true);
  refuses(tooMany, 'invalid-value', 'tile count');
  // A message that ends inside its records length, and one without rectangles.
  refuses(sent('tiles').subarray(0, 33), 'truncated');
  refuses(Uint8Array.from(tooMany).fill(0, 13, 15), 'invalid-value', 'tile count');
  // A custom cursor (shape 255) whose image is over its bound, and one at it.
  const cursor = new Uint8Array(22 + MAX_CURSOR_IMAGE + 1);
  cursor.set(sent('cursor'));
  cursor[17] = 255;
  refuses(cursor, 'too-large', 'cursor image');
  const cursorAtBound = cursor.subarray(0, 22 + MAX_CURSOR_IMAGE);
  assert.equal((decode(cursorAtBound) as { image: Uint8Array }).image.length, MAX_CURSOR_IMAGE);
  // A cursor shape the protocol does not number, a named shape with an image, and a custom one without.
  const shaped = (shape: number, ...image: number[]) =>
    bytes(0x05, 7, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, shape, 0, 0, 0, 0, ...image);
  const qoif = [0x71, 0x6f, 0x69, 0x66];
  for (const ok of [shaped(0), shaped(34), shaped(255, ...qoif)]) assert.equal(decode(ok).type, 'cursor');
  for (const bad of [shaped(35), shaped(254), shaped(1, ...qoif), shaped(255)]) refuses(bad, 'invalid-value', 'cursor shape');
  // The message bound comes first, before any field.
  refuses(new Uint8Array(MAX_SERVER_MESSAGE + 1), 'too-large');
  const largest = new Uint8Array(MAX_SERVER_MESSAGE);
  largest.set(sent('video'));
  assert.equal((decode(largest) as { payload: Uint8Array }).payload.length, MAX_SERVER_MESSAGE - 30);
});

test('UTF-8 is validated like Rust String::from_utf8', () => {
  const copied = (...tail: number[]): Uint8Array => bytes(0x0d, 3, 0, 0, 0, ...tail);
  // Overlong encodings, surrogates, and truncated sequences are refused; a byte-order mark is kept as text.
  refuses(copied(0xc0, 0xaf), 'invalid-value', 'utf-8');
  refuses(copied(0xed, 0xa0, 0x80), 'invalid-value', 'utf-8');
  refuses(copied(0xe2, 0x82), 'invalid-value', 'utf-8');
  refuses(copied(0xf4, 0x90, 0x80, 0x80), 'invalid-value', 'utf-8');
  assert.deepEqual(decode(copied(0xef, 0xbb, 0xbf, 0x61)), { type: 'copied', leaseEpoch: 3, text: '\u{feff}a' });
  assert.deepEqual(decode(copied()), { type: 'copied', leaseEpoch: 3, text: '' });
});

test('wheel deltas and u64 fields cover their full ranges', () => {
  const wheel = encode({ type: 'wheel', leaseEpoch: 0xffff_ffff, x: 0xffff, y: 0, dx: -0x8000, dy: 0x7fff });
  assert.equal(hex(wheel), '11ffffffffffff00000080ff7f');
  assert.equal(hex(encode({ type: 'time-ping', clientTimeUs: 0xffff_ffff_ffff_ffffn })), '09ffffffffffffffff');
});

test('the encoder refuses values outside their wire width instead of wrapping', () => {
  assert.throws(() => encode({ type: 'wheel', leaseEpoch: 1, x: 0, y: 0, dx: 0x8000, dy: 0 }), RangeError);
  assert.throws(() => encode({ type: 'pointer', leaseEpoch: -1, kind: PointerKind.Move, button: 0, x: 0, y: 0 }), RangeError);
  assert.throws(() => encode({ type: 'time-ping', clientTimeUs: -1n }), RangeError);
  assert.throws(() => encode({ type: 'pointer', leaseEpoch: 1, kind: PointerKind.Move, button: 0, x: 1.5, y: 0 }), RangeError);
});

test('messages larger than the writer\'s first buffer encode whole, at every field', () => {
  const head = (...b: number[]) => hex(Uint8Array.from(b));
  const text = 'é'.repeat(30_000);
  const paste = encode({ type: 'clipboard', leaseEpoch: 3, text });
  assert.equal(hex(paste), head(0x13, 3, 0, 0, 0) + hex(new TextEncoder().encode(text)));
  const data = Uint8Array.from({ length: 61_440 }, (_, i) => i & 255);
  const piece = encode({ type: 'upload-data', leaseEpoch: 3, upload: 1, offset: 1n << 40n, data });
  assert.equal(hex(piece), head(0x16, 3, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0) + hex(data));
  const uploads = Array.from({ length: 64 }, (_, i) => i);
  const many = encode({ type: 'file-choose', leaseEpoch: 3, request: 9, uploads });
  assert.equal(hex(many), head(0x17, 3, 0, 0, 0, 9, 0, 0, 0) + uploads.map((u) => head(u, 0, 0, 0)).join(''));
});
