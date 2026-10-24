import assert from 'node:assert/strict';
import { test } from 'node:test';
import vectors from '../../protocol/vectors/v1.txt';
import {
  CODEC_AV1,
  CODEC_H264,
  CAMERA_SIDE_MIN,
  CameraBorder,
  Codec,
  decode,
  direction,
  encode,
  type ErrorKind,
  KeyKind,
  LeaseHolder,
  MAX_AUDIO_PACKET,
  MAX_CLIENT_MESSAGE,
  MAX_CLIPBOARD,
  MAX_CHOSEN,
  MAX_COPIED,
  MAX_FILE_NAME,
  FILE_CHUNK,
  MAX_CURSOR_IMAGE,
  MAX_SERVER_MESSAGE,
  MAX_TEXT,
  MAX_TILES,
  type Message,
  PointerKind,
  ProtocolError,
  RecordPhase,
  MAX_RECORD_PICTURE,
  VERSION,
} from '../src/protocol.ts';

const bytes = (...b: number[]): Uint8Array => Uint8Array.from(b);
const rect = { x: 64, y: 128, w: 640, h: 400 };

const LAYOUT = { x: 7_400, y: 6_800, side: 2_400, radius: 100, border: CameraBorder.Coral };

// The values protocol/tests/vectors.rs encodes into vectors/v1.txt.
const samples: Record<string, Message> = {
  hello: { type: 'hello', version: VERSION, codecs: CODEC_AV1 | CODEC_H264 },
  config: {
    type: 'config',
    version: VERSION,
    epoch: 7,
    width: 1280,
    height: 800,
    codec: Codec.H264,
    maxFps: 60,
  },
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
  'file-data': { type: 'file-data', leaseEpoch: 3, file: 7, offset: 65_536n, data: bytes(1, 2, 3) },
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
  cursor: { type: 'cursor', epoch: 7, x: 300, y: 200, shape: 1, hotX: 0, hotY: 0, image: bytes() },
  'keyframe-request': { type: 'keyframe-request', epoch: 7 },
  'server-stats': {
    type: 'server-stats',
    serverTimeUs: 5_000_000n,
    frames: 60,
    bytes: 250_000,
    encodeUsP50: 4_000,
    encodeUsP95: 9_000,
  },
  'client-stats': { type: 'client-stats', presented: 59, decodeUsP50: 1_500, hardware: true },
  'time-ping': { type: 'time-ping', clientTimeUs: 42n },
  'time-pong': { type: 'time-pong', clientTimeUs: 42n, serverTimeUs: 99n },
  lease: { type: 'lease', leaseEpoch: 3, holder: LeaseHolder.You },
  pointer: { type: 'pointer', leaseEpoch: 3, kind: PointerKind.Down, button: 0, x: 10, y: 20 },
  wheel: { type: 'wheel', leaseEpoch: 3, x: 10, y: 20, dx: 0, dy: -120 },
  key: { type: 'key', leaseEpoch: 3, kind: KeyKind.Down, code: 30, text: '' },
  'key-text': { type: 'key', leaseEpoch: 3, kind: KeyKind.Text, code: 0, text: 'ção' },
  clipboard: { type: 'clipboard', leaseEpoch: 3, text: 'hello' },
  'file-read': { type: 'file-read', leaseEpoch: 3, file: 7, offset: 65_536n },
  'file-cancel': { type: 'file-cancel', leaseEpoch: 3, file: 7 },
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
  'record-state': {
    type: 'record-state',
    leaseEpoch: 3,
    recording: 2,
    phase: RecordPhase.Recording,
    file: 9,
    seconds: 75,
    bytes: 5_000_000_000n,
  },
};

function hex(b: Uint8Array): string {
  return Array.from(b, (v) => v.toString(16).padStart(2, '0')).join('');
}

function unhex(s: string): Uint8Array {
  assert(/^([0-9a-f]{2})*$/.test(s), `not hex: ${s}`);
  return Uint8Array.from(s.match(/../g) ?? [], (h) => parseInt(h, 16));
}

const golden = vectors
  .split('\n')
  .filter((line) => line !== '')
  .map((line) => {
    const [name, value, ...rest] = line.split(' ');
    assert(name !== undefined && value !== undefined && rest.length === 0, `malformed vector line: ${line}`);
    return { name, bytes: unhex(value) };
  });

function refuses(b: Uint8Array, dir: 'client-to-server' | 'server-to-client', kind: ErrorKind, detail?: string): void {
  assert.throws(
    () => decode(b, dir),
    (e: unknown) => e instanceof ProtocolError && e.kind === kind && (detail === undefined || e.detail === detail),
  );
}

test('every golden vector decodes to its value and re-encodes byte for byte', () => {
  assert.deepEqual(
    golden.map((v) => v.name),
    Object.keys(samples),
  );
  for (const vector of golden) {
    const expected = samples[vector.name]!;
    const decoded = decode(vector.bytes, direction(expected));
    assert.deepEqual(decoded, expected, vector.name);
    assert.equal(hex(encode(decoded)), hex(vector.bytes), vector.name);
    assert.equal(hex(encode(expected)), hex(vector.bytes), vector.name);
  }
});

test('directions match the Rust protocol', () => {
  const c2s = [
    'hello',
    'keyframe-request',
    'client-stats',
    'time-ping',
    'pointer',
    'wheel',
    'key',
    'key-text',
    'clipboard',
    'file-read',
    'file-cancel',
    'upload-start',
    'upload-data',
    'file-choose',
    'file-drop',
    'file-drag',
    'file-drag-end',
    'record-start',
    'record-camera',
    'record-microphone',
    'record-stop',
    'record-layout',
  ];
  for (const [name, message] of Object.entries(samples)) {
    assert.equal(direction(message), c2s.includes(name) ? 'client-to-server' : 'server-to-client', name);
  }
});

test('refuses malformed messages exactly as protocol/tests/vectors.rs', () => {
  const c2s = 'client-to-server';
  const s2c = 'server-to-client';
  refuses(bytes(), c2s, 'empty');
  refuses(bytes(0x7f), c2s, 'unknown-kind', '0x7f');
  refuses(bytes(0x06, 1, 0, 0), c2s, 'truncated');
  refuses(bytes(0x06, 1, 0, 0, 0, 9), c2s, 'trailing-bytes');
  // A server message arriving from a client is refused, and the other way round.
  refuses(encode(samples.config!), c2s, 'invalid-value', 'direction');
  refuses(encode(samples.hello!), s2c, 'invalid-value', 'direction');
  // Invalid UTF-8, an empty text key, a text on a physical key, a bad button, and an oversized paste.
  refuses(bytes(0x13, 3, 0, 0, 0, 0xff), c2s, 'invalid-value', 'utf-8');
  refuses(bytes(0x12, 3, 0, 0, 0, 2, 0, 0), c2s, 'invalid-value', 'key');
  refuses(bytes(0x12, 3, 0, 0, 0, 0, 0, 0), c2s, 'invalid-value', 'key');
  refuses(bytes(0x12, 3, 0, 0, 0, 2, 30, 0, 0x61), c2s, 'invalid-value', 'key');
  refuses(bytes(0x12, 3, 0, 0, 0, 0, 30, 0, 0x61), c2s, 'invalid-value', 'key');
  refuses(bytes(0x10, 3, 0, 0, 0, 1, 9, 0, 0, 0, 0), c2s, 'invalid-value', 'button');
  const paste = new Uint8Array(5 + MAX_CLIPBOARD + 1).fill(0x61);
  paste.set([0x13, 3, 0, 0, 0]);
  refuses(paste, c2s, 'too-large');
  // A hello of another version and one with an unknown codec bit; a zero-size configuration.
  refuses(bytes(0x01, VERSION + 1, 0, 3), c2s, 'invalid-value', 'version');
  refuses(bytes(0x01, VERSION, 0, 0x04), c2s, 'invalid-value', 'codec set');
  refuses(bytes(0x02, VERSION, 0, 7, 0, 0, 0, 0, 0, 0x20, 3, 2, 60), s2c, 'invalid-value', 'configuration');
});

test('refuses out-of-range values of every closed field', () => {
  const s2c = 'server-to-client';
  const c2s = 'client-to-server';
  const config = encode(samples.config!);
  refuses(Uint8Array.from(config).fill(3, 11, 12), s2c, 'invalid-value', 'codec');
  refuses(Uint8Array.from(config).fill(0, 12, 13), s2c, 'invalid-value', 'configuration');
  // A tile wider than MAX_TILE_SIDE (the rectangle's width sits at bytes 19-20 of the tiles sample).
  const wide = Uint8Array.from(encode(samples.tiles!));
  wide.set([1, 1], 19);
  refuses(wide, s2c, 'invalid-value', 'tile size');
  // A records length shorter than all solid records or longer than all RGB ones (it follows the two rectangles).
  for (const rawLen of [7, 64 * 64 * 3 + 1 + 7 + 1]) {
    const bad = Uint8Array.from(encode(samples.tiles!));
    new DataView(bad.buffer).setUint32(31, rawLen, true);
    refuses(bad, s2c, 'invalid-value', 'tile records length');
  }
  refuses(bytes(0x0b, 3, 0, 0, 0, 3), s2c, 'invalid-value', 'lease holder');
  refuses(bytes(0x10, 3, 0, 0, 0, 3, 0, 0, 0, 0, 0), c2s, 'invalid-value', 'pointer kind');
  refuses(bytes(0x12, 3, 0, 0, 0, 3, 30, 0), c2s, 'invalid-value', 'key kind');
  refuses(bytes(0x08, 1, 0, 0, 0, 1, 0, 0, 0, 2), c2s, 'invalid-value', 'flag');
  const video = encode(samples.video!);
  refuses(Uint8Array.from(video).fill(2, 21, 22), s2c, 'invalid-value', 'flag');
  refuses(Uint8Array.from(video).fill(0, 17, 19), s2c, 'invalid-value', 'rectangle');
  refuses(video.subarray(0, 20), s2c, 'truncated');
});

test('refuses tiles, cursor images, and texts over their bounds', () => {
  const s2c = 'server-to-client';
  const c2s = 'client-to-server';
  // An upload without a name or with an empty piece; upload lists not of whole ids, too long, or an empty drop.
  refuses(bytes(0x15, 3, 0, 0, 0, 4, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0), c2s, 'invalid-value', 'file name');
  refuses(bytes(0x16, 3, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0), c2s, 'invalid-value', 'upload piece size');
  refuses(bytes(0x17, 3, 0, 0, 0, 9, 0, 0, 0, 4, 0), c2s, 'invalid-value', 'upload list');
  const many = new Uint8Array(9 + 4 * (MAX_CHOSEN + 1));
  many.set([0x17, 3, 0, 0, 0, 9, 0, 0, 0]);
  refuses(many, c2s, 'invalid-value', 'upload list');
  refuses(bytes(0x18, 3, 0, 0, 0, 0, 0, 0, 0), c2s, 'invalid-value', 'upload list');
  // A drag of no files, a type outside the lowercase type/subtype form, and a type longer than its bytes.
  refuses(bytes(0x21, 3, 0, 0, 0, 0, 0, 0, 0, 0), c2s, 'invalid-value', 'drag types');
  for (const bad of ['Image/png', 'image', 'image/', '/png', 'image/png/x', 'text/plain; charset=utf-8']) {
    const t = new TextEncoder().encode(bad);
    refuses(Uint8Array.from([0x21, 3, 0, 0, 0, 0, 0, 0, 0, 1, t.length, ...t]), c2s, 'invalid-value', 'media type');
  }
  refuses(bytes(0x21, 3, 0, 0, 0, 0, 0, 0, 0, 1, 3, 0x61, 0x2f), c2s, 'truncated');
  // A file offer without a name or with a name over its bound; a file piece empty or over its bound.
  const offer = bytes(0x0e, 3, 0, 0, 0, 7, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0);
  // A size said to be unknown must be zero.
  refuses(bytes(0x0e, 3, 0, 0, 0, 7, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0x61), s2c, 'invalid-value', 'file size');
  refuses(offer, s2c, 'invalid-value', 'file name');
  const named = new Uint8Array(offer.length + MAX_FILE_NAME + 1).fill(0x61);
  named.set(offer);
  refuses(named, s2c, 'too-large');
  const piece = bytes(0x0f, 3, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
  refuses(piece, s2c, 'invalid-value', 'file piece size');
  const big = new Uint8Array(piece.length + FILE_CHUNK + 1);
  big.set(piece);
  refuses(big, s2c, 'invalid-value', 'file piece size');
  // A download ticket of another length.
  for (const [length, kind] of [[31, 'truncated'], [33, 'trailing-bytes']] as const) {
    const link = new Uint8Array(9 + length);
    link.set([0x20, 3, 0, 0, 0, 7, 0, 0, 0]);
    refuses(link, s2c, kind);
  }
  // Copied text over its bound, not UTF-8, or from a client.
  const copied = new Uint8Array(5 + MAX_COPIED + 1).fill(0x61);
  copied.set([0x0d, 3, 0, 0, 0]);
  refuses(copied, s2c, 'too-large');
  refuses(bytes(0x0d, 3, 0, 0, 0, 0xff), s2c, 'invalid-value', 'utf-8');
  refuses(bytes(0x0d, 3, 0, 0, 0, 0x61), c2s, 'invalid-value', 'direction');
  // An empty or over-long audio packet, and audio from a client.
  refuses(bytes(0x0c, 0, 0, 0, 0, 0, 0, 0, 0), s2c, 'invalid-value', 'audio packet size');
  const longAudio = new Uint8Array(9 + MAX_AUDIO_PACKET + 1).fill(0xfc);
  longAudio.set([0x0c, 0, 0, 0, 0, 0, 0, 0, 0]);
  refuses(longAudio, s2c, 'invalid-value', 'audio packet size');
  refuses(encode(samples.audio!), c2s, 'invalid-value', 'direction');
  const tooMany = new Uint8Array(15);
  tooMany.set([0x04, 7, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
  new DataView(tooMany.buffer).setUint16(13, MAX_TILES + 1, true);
  refuses(tooMany, s2c, 'invalid-value', 'tile count');
  // A message that ends inside its records length, and one without rectangles.
  const tiles = encode(samples.tiles!);
  refuses(tiles.subarray(0, 33), s2c, 'truncated');
  refuses(Uint8Array.from(tooMany).fill(0, 13, 15), s2c, 'invalid-value', 'tile count');
  const cursor = new Uint8Array(14 + MAX_CURSOR_IMAGE + 1);
  cursor.set(encode({ ...samples.cursor!, shape: 255 } as Message));
  refuses(cursor, s2c, 'too-large', 'cursor image');
  const cursorAtBound = cursor.subarray(0, 14 + MAX_CURSOR_IMAGE);
  assert.equal((decode(cursorAtBound, s2c) as { image: Uint8Array }).image.length, MAX_CURSOR_IMAGE);
  // A cursor shape the protocol does not number, a named shape with an image, and a custom one without.
  const shaped = (shape: number, ...image: number[]) => bytes(0x05, 7, 0, 0, 0, 0, 0, 0, 0, shape, 0, 0, 0, 0, ...image);
  const qoif = [0x71, 0x6f, 0x69, 0x66];
  for (const ok of [shaped(0), shaped(34), shaped(255, ...qoif)]) assert.equal(decode(ok, s2c).type, 'cursor');
  for (const bad of [shaped(35), shaped(254), shaped(1, ...qoif), shaped(255)]) refuses(bad, s2c, 'invalid-value', 'cursor shape');
  const text = new Uint8Array(8 + MAX_TEXT + 1).fill(0x61);
  text.set([0x12, 3, 0, 0, 0, 2, 0, 0]);
  refuses(text, c2s, 'too-large');
  assert.equal((decode(text.subarray(0, 8 + MAX_TEXT), c2s) as { text: string }).text.length, MAX_TEXT);
  // The message bounds of each direction come first, before any field.
  refuses(new Uint8Array(MAX_CLIENT_MESSAGE + 1), c2s, 'too-large');
  refuses(new Uint8Array(MAX_SERVER_MESSAGE + 1), s2c, 'too-large');
  const largest = new Uint8Array(MAX_SERVER_MESSAGE);
  largest.set(encode(samples.video!));
  assert.equal((decode(largest, s2c) as { payload: Uint8Array }).payload.length, MAX_SERVER_MESSAGE - 30);
});

test('UTF-8 is validated like Rust String::from_utf8', () => {
  const c2s = 'client-to-server';
  const clip = (...tail: number[]): Uint8Array => bytes(0x13, 3, 0, 0, 0, ...tail);
  // Overlong encodings, surrogates, and truncated sequences are refused; a byte-order mark is kept as text.
  refuses(clip(0xc0, 0xaf), c2s, 'invalid-value', 'utf-8');
  refuses(clip(0xed, 0xa0, 0x80), c2s, 'invalid-value', 'utf-8');
  refuses(clip(0xe2, 0x82), c2s, 'invalid-value', 'utf-8');
  refuses(clip(0xf4, 0x90, 0x80, 0x80), c2s, 'invalid-value', 'utf-8');
  assert.deepEqual(decode(clip(0xef, 0xbb, 0xbf, 0x61), c2s), { type: 'clipboard', leaseEpoch: 3, text: '﻿a' });
  assert.deepEqual(decode(clip(), c2s), { type: 'clipboard', leaseEpoch: 3, text: '' });
});

test('wheel deltas and u64 fields cover their full ranges', () => {
  const wheel: Message = { type: 'wheel', leaseEpoch: 0xffff_ffff, x: 0xffff, y: 0, dx: -0x8000, dy: 0x7fff };
  assert.deepEqual(decode(encode(wheel), 'client-to-server'), wheel);
  const ping: Message = { type: 'time-ping', clientTimeUs: 0xffff_ffff_ffff_ffffn };
  assert.deepEqual(decode(encode(ping), 'client-to-server'), ping);
});

test('the encoder refuses values outside their wire width instead of wrapping', () => {
  assert.throws(() => encode({ type: 'wheel', leaseEpoch: 1, x: 0, y: 0, dx: 0x8000, dy: 0 }), RangeError);
  assert.throws(() => encode({ type: 'pointer', leaseEpoch: -1, kind: PointerKind.Move, button: 0, x: 0, y: 0 }), RangeError);
  assert.throws(() => encode({ type: 'time-ping', clientTimeUs: -1n }), RangeError);
  assert.throws(() => encode({ type: 'pointer', leaseEpoch: 1, kind: PointerKind.Move, button: 0, x: 1.5, y: 0 }), RangeError);
});

test('messages larger than the writer\'s first buffer encode whole, at every field', () => {
  const c2s = 'client-to-server';
  const text = 'é'.repeat(30_000);
  const paste: Message = { type: 'clipboard', leaseEpoch: 3, text };
  assert.deepEqual(decode(encode(paste), c2s), paste);
  const data = Uint8Array.from({ length: 61_440 }, (_, i) => i & 255);
  const piece: Message = { type: 'upload-data', leaseEpoch: 3, upload: 1, offset: 1n << 40n, data };
  assert.deepEqual(decode(encode(piece), c2s), piece);
  const many: Message = { type: 'file-choose', leaseEpoch: 3, request: 9, uploads: Array.from({ length: 64 }, (_, i) => i) };
  assert.deepEqual(decode(encode(many), c2s), many);
});

test('refuses malformed recording messages exactly as protocol/tests/vectors.rs', () => {
  const c2s = 'client-to-server';
  const s2c = 'server-to-client';
  const start = [0x23, 3, 0, 0, 0, 2, 0, 0, 0, 0x10, 0x27, 0, 0, 0xa0, 0x0f, 100, 0, 3];
  assert.equal(decode(bytes(...start), c2s).type, 'record-start', 'x at the right edge, the largest side, a circle');
  const invalid = (at: number, value: number, why: string) => {
    const bad = [...start];
    bad[at] = value;
    refuses(bytes(...bad), c2s, 'invalid-value', why);
  };
  invalid(9, 0x11, 'camera position');
  invalid(13, 0xa1, 'camera side');
  invalid(14, 0x03, 'camera side');
  invalid(15, 101, 'camera radius');
  invalid(16, 3, 'camera border');
  invalid(17, 8, 'recording flags');
  const smallest = [...start];
  smallest.splice(13, 2, CAMERA_SIDE_MIN & 0xff, CAMERA_SIDE_MIN >> 8);
  assert.equal(decode(bytes(...smallest), c2s).type, 'record-start', 'the smallest side');
  refuses(bytes(0x28, 3, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0x11, 0x27, 0x60, 0x09, 0, 0), c2s, 'invalid-value', 'camera position');
  const header = [0x24, 3, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
  refuses(bytes(...header), c2s, 'invalid-value', 'camera picture size');
  const picture = new Uint8Array(header.length + MAX_RECORD_PICTURE + 1);
  picture.set(header);
  refuses(picture, c2s, 'invalid-value', 'camera picture size');
  refuses(bytes(0x25, 3, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0), c2s, 'invalid-value', 'microphone packet size');
  const state = encode(samples['record-state']!);
  refuses(state, c2s, 'invalid-value', 'direction');
  refuses(Uint8Array.from(state).fill(3, 9, 10), s2c, 'invalid-value', 'recording phase');
});
