// The easi.run wire protocol, version `VERSION`: an exact mirror of protocol/src/lib.rs.
//
// One message is one WebSocket binary frame: a one-byte kind followed by fixed little-endian fields. The set is
// closed, and decoding refuses an unknown kind, a short or over-long body, an out-of-range value, invalid UTF-8, or
// a message from the wrong side, with the same error (and in the same order) as the Rust decoder.

export const VERSION = 1;
export const MAX_CLIPBOARD = 64 * 1024;
export const MAX_SERVER_MESSAGE = 2 * 1024 * 1024;
export const MAX_CLIENT_MESSAGE = MAX_CLIPBOARD + 16;
export const MAX_TEXT = 1024;
/** Largest piece of a file one file-data message carries, and the largest file name, in bytes. */
export const FILE_CHUNK = 64 * 1024;
export const MAX_FILE_NAME = 255;
/** Length of a download ticket, in bytes. */
const FILE_TICKET = 32;
/** Largest piece of an upload, the bytes a client may send ahead of the last acknowledgement, and most uploads named. */
export const UPLOAD_CHUNK = 60 * 1024;
export const UPLOAD_WINDOW = 256 * 1024;
export const MAX_CHOSEN = 64;
/** Largest media type a `file-drag` names, in bytes. */
const MAX_DRAG_TYPE = 127;
/** Largest text the browser may copy to the person, in bytes. */
export const MAX_COPIED = 1024 * 1024;
export const MAX_TILES = 4096;
/** Largest side of a tile's rectangle, in pixels. */
export const MAX_TILE_SIDE = 256;
/** Largest side of the output a configuration announces, in pixels. */
const MAX_OUTPUT_SIDE = 4096;
/** Largest inflated size of one tiles message's records, in bytes. */
export const MAX_TILES_RAW = 8 * 1024 * 1024;
export const MAX_CURSOR_IMAGE = 64 * 1024;
/** Largest Opus packet (RFC 6716, section 3.4), in bytes. */
export const MAX_AUDIO_PACKET = 1275;
/** Largest camera picture a `record-camera` message carries, in bytes. */
export const MAX_RECORD_PICTURE = 60 * 1024;
/** The duration of every audio packet, in microseconds, and its sample rate and channel count. */
export const AUDIO_FRAME_US = 20_000;
export const AUDIO_SAMPLE_RATE = 48_000;
export const AUDIO_CHANNELS = 2;
export const H264_CODEC_STRING = 'avc1.42E028';
export const AV1_CODEC_STRING = 'av01.0.08M.08';

/** Bit set of the video codecs a client can decode. */
export const CODEC_AV1 = 1 << 0;
export const CODEC_H264 = 1 << 1;
const KNOWN_CODECS = CODEC_AV1 | CODEC_H264;

export const Codec = { None: 0, Av1: 1, H264: 2 } as const;
export type Codec = (typeof Codec)[keyof typeof Codec];
export const PointerKind = { Move: 0, Down: 1, Up: 2 } as const;
export type PointerKind = (typeof PointerKind)[keyof typeof PointerKind];
export const KeyKind = { Down: 0, Up: 1, Text: 2 } as const;
export type KeyKind = (typeof KeyKind)[keyof typeof KeyKind];
/** Steps of a camera position or side: a share of the screen, from 0 to this. */
export const LAYOUT_UNITS = 10_000;
/** The camera's smallest and largest side, in `LAYOUT_UNITS` of the screen's width. */
export const CAMERA_SIDE_MIN = 1_000;
export const CAMERA_SIDE_MAX = 4_000;
/** The camera's roundest corners, in percent of half its side: a circle. */
export const CAMERA_RADIUS_MAX = 100;
export const CameraBorder = { White: 0, Coral: 1, None: 2 } as const;
export type CameraBorder = (typeof CameraBorder)[keyof typeof CameraBorder];
export const RecordPhase = { Recording: 0, Ended: 1, Failed: 2 } as const;
export type RecordPhase = (typeof RecordPhase)[keyof typeof RecordPhase];
/**
 * Where the camera is in a recording, and how it looks: a square whose top-left corner is at `x` and `y` (in
 * `LAYOUT_UNITS` of the screen's width and height), of `side` (in `LAYOUT_UNITS` of the screen's width), with its
 * corners rounded by `radius` percent of half its side (0 square, 100 a circle).
 */
export interface CameraLayout {
  x: number;
  y: number;
  side: number;
  radius: number;
  border: CameraBorder;
}
export const LeaseHolder = { Agent: 0, You: 1, Other: 2 } as const;
export type LeaseHolder = (typeof LeaseHolder)[keyof typeof LeaseHolder];

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type Message =
  | { type: 'hello'; version: number; codecs: number }
  | { type: 'config'; version: number; epoch: number; width: number; height: number; codec: Codec; maxFps: number }
  | { type: 'video'; epoch: number; revision: bigint; rect: Rect; keyframe: boolean; timestampUs: bigint; payload: Uint8Array }
  | { type: 'audio'; timestampUs: bigint; payload: Uint8Array }
  | { type: 'copied'; leaseEpoch: number; text: string }
  | { type: 'file-offer'; leaseEpoch: number; file: number; size: bigint | null; name: string }
  | { type: 'file-link'; leaseEpoch: number; file: number; ticket: Uint8Array }
  | { type: 'file-data'; leaseEpoch: number; file: number; offset: bigint; data: Uint8Array }
  | { type: 'file-end'; leaseEpoch: number; file: number; size: bigint }
  | { type: 'file-read'; leaseEpoch: number; file: number; offset: bigint }
  | { type: 'file-cancel'; leaseEpoch: number; file: number }
  | { type: 'file-gone'; leaseEpoch: number; file: number }
  | { type: 'file-request'; leaseEpoch: number; request: number; multiple: boolean }
  | { type: 'file-request-end'; leaseEpoch: number; request: number }
  | { type: 'upload-ack'; leaseEpoch: number; upload: number; received: bigint }
  | { type: 'upload-done'; leaseEpoch: number; upload: number; stored: boolean }
  | { type: 'upload-start'; leaseEpoch: number; upload: number; size: bigint; name: string }
  | { type: 'upload-data'; leaseEpoch: number; upload: number; offset: bigint; data: Uint8Array }
  | { type: 'file-choose'; leaseEpoch: number; request: number; uploads: number[] }
  | { type: 'file-drop'; leaseEpoch: number; x: number; y: number; uploads: number[] }
  | { type: 'file-drag'; leaseEpoch: number; x: number; y: number; types: string[] }
  | { type: 'file-drag-end'; leaseEpoch: number }
  | { type: 'tiles'; epoch: number; revision: bigint; rects: Rect[]; rawLen: number; data: Uint8Array }
  | { type: 'cursor'; epoch: number; x: number; y: number; shape: number; hotX: number; hotY: number; image: Uint8Array }
  | { type: 'keyframe-request'; epoch: number }
  | { type: 'server-stats'; serverTimeUs: bigint; frames: number; bytes: number; encodeUsP50: number; encodeUsP95: number }
  | { type: 'client-stats'; presented: number; decodeUsP50: number; hardware: boolean }
  | { type: 'time-ping'; clientTimeUs: bigint }
  | { type: 'time-pong'; clientTimeUs: bigint; serverTimeUs: bigint }
  | { type: 'lease'; leaseEpoch: number; holder: LeaseHolder }
  | { type: 'pointer'; leaseEpoch: number; kind: PointerKind; button: number; x: number; y: number }
  | { type: 'wheel'; leaseEpoch: number; x: number; y: number; dx: number; dy: number }
  | { type: 'key'; leaseEpoch: number; kind: KeyKind; code: number; text: string }
  | { type: 'clipboard'; leaseEpoch: number; text: string }
  | {
      type: 'record-start';
      leaseEpoch: number;
      recording: number;
      layout: CameraLayout;
      camera: boolean;
      microphone: boolean;
      pageSound: boolean;
    }
  | { type: 'record-camera'; leaseEpoch: number; recording: number; timestampUs: bigint; jpeg: Uint8Array }
  | { type: 'record-microphone'; leaseEpoch: number; recording: number; timestampUs: bigint; payload: Uint8Array }
  | { type: 'record-stop'; leaseEpoch: number; recording: number }
  | { type: 'record-layout'; leaseEpoch: number; recording: number; layout: CameraLayout }
  | { type: 'record-state'; leaseEpoch: number; recording: number; phase: RecordPhase; file: number; seconds: number; bytes: bigint };

export type MessageOf<T extends Message['type']> = Extract<Message, { type: T }>;

type Direction = 'client-to-server' | 'server-to-client';

const KIND = {
  hello: 0x01,
  config: 0x02,
  video: 0x03,
  tiles: 0x04,
  cursor: 0x05,
  'keyframe-request': 0x06,
  'server-stats': 0x07,
  'client-stats': 0x08,
  'time-ping': 0x09,
  'time-pong': 0x0a,
  lease: 0x0b,
  audio: 0x0c,
  copied: 0x0d,
  'file-offer': 0x0e,
  'file-data': 0x0f,
  'file-request': 0x1a,
  'upload-ack': 0x1b,
  'upload-done': 0x1c,
  'file-gone': 0x1d,
  'file-request-end': 0x1e,
  'file-end': 0x1f,
  'file-link': 0x20,
  pointer: 0x10,
  wheel: 0x11,
  key: 0x12,
  clipboard: 0x13,
  'file-read': 0x14,
  'upload-start': 0x15,
  'upload-data': 0x16,
  'file-choose': 0x17,
  'file-drop': 0x18,
  'file-cancel': 0x19,
  'file-drag': 0x21,
  'file-drag-end': 0x22,
  'record-start': 0x23,
  'record-camera': 0x24,
  'record-microphone': 0x25,
  'record-stop': 0x26,
  'record-state': 0x27,
  'record-layout': 0x28,
} as const satisfies Record<Message['type'], number>;

const CLIENT_TO_SERVER: ReadonlySet<Message['type']> = new Set([
  'hello',
  'keyframe-request',
  'client-stats',
  'time-ping',
  'pointer',
  'wheel',
  'key',
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
]);

export function direction(message: Message): Direction {
  return CLIENT_TO_SERVER.has(message.type) ? 'client-to-server' : 'server-to-client';
}

export type ErrorKind = 'empty' | 'unknown-kind' | 'truncated' | 'trailing-bytes' | 'too-large' | 'invalid-value';

/** A refused message; `detail` is the Rust `InvalidValue` text or the unknown kind byte. */
export class ProtocolError extends Error {
  constructor(
    readonly kind: ErrorKind,
    readonly detail?: string,
  ) {
    super(detail === undefined ? kind : `${kind}: ${detail}`);
    this.name = 'ProtocolError';
  }
}

const utf8Encoder = new TextEncoder();
// `ignoreBOM` keeps a leading U+FEFF as text, as Rust's `String::from_utf8` does.
const utf8Decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

class Writer {
  private bytes: Uint8Array<ArrayBuffer> = new Uint8Array(64);
  private view = new DataView(this.bytes.buffer);
  private len = 0;

  private reserve(n: number): number {
    if (this.len + n > this.bytes.length) {
      const grown = new Uint8Array(Math.max(this.bytes.length * 2, this.len + n));
      grown.set(this.bytes.subarray(0, this.len));
      this.bytes = grown;
      this.view = new DataView(grown.buffer);
    }
    const at = this.len;
    this.len += n;
    return at;
  }

  private int(value: number, min: number, max: number): number {
    if (!Number.isInteger(value) || value < min || value > max) {
      throw new RangeError(`value ${value} outside [${min}, ${max}]`);
    }
    return value;
  }

  // Each writer reserves before it reads `bytes` or `view`: reserving may replace them with larger ones, and a call like
  // `this.view.setUint8(this.reserve(1), v)` would read the old one first and write past its end.
  u8(v: number): void {
    const at = this.reserve(1);
    this.view.setUint8(at, this.int(v, 0, 0xff));
  }
  bool(v: boolean): void {
    this.u8(v ? 1 : 0);
  }
  layout(l: CameraLayout): void {
    this.u16(l.x);
    this.u16(l.y);
    this.u16(l.side);
    this.u8(l.radius);
    this.u8(l.border);
  }

  u16(v: number): void {
    const at = this.reserve(2);
    this.view.setUint16(at, this.int(v, 0, 0xffff), true);
  }
  i16(v: number): void {
    const at = this.reserve(2);
    this.view.setInt16(at, this.int(v, -0x8000, 0x7fff), true);
  }
  u32(v: number): void {
    const at = this.reserve(4);
    this.view.setUint32(at, this.int(v, 0, 0xffff_ffff), true);
  }
  u64(v: bigint): void {
    if (v < 0n || v > 0xffff_ffff_ffff_ffffn) throw new RangeError(`value ${v} outside u64`);
    const at = this.reserve(8);
    this.view.setBigUint64(at, v, true);
  }
  rect(r: Rect): void {
    this.u16(r.x);
    this.u16(r.y);
    this.u16(r.w);
    this.u16(r.h);
  }
  raw(b: Uint8Array): void {
    const at = this.reserve(b.length);
    this.bytes.set(b, at);
  }
  finish(): Uint8Array<ArrayBuffer> {
    return this.bytes.slice(0, this.len);
  }
}

/** Encodes one message. Integer fields outside their wire width throw instead of wrapping. */
export function encode(m: Message): Uint8Array<ArrayBuffer> {
  const w = new Writer();
  w.u8(KIND[m.type]);
  switch (m.type) {
    case 'hello':
      w.u16(m.version);
      w.u8(m.codecs);
      break;
    case 'config':
      w.u16(m.version);
      w.u32(m.epoch);
      w.u16(m.width);
      w.u16(m.height);
      w.u8(m.codec);
      w.u8(m.maxFps);
      break;
    case 'video':
      w.u32(m.epoch);
      w.u64(m.revision);
      w.rect(m.rect);
      w.bool(m.keyframe);
      w.u64(m.timestampUs);
      w.raw(m.payload);
      break;
    case 'copied':
      w.u32(m.leaseEpoch);
      {
        const text = utf8Encoder.encode(m.text);
        if (text.length > MAX_COPIED) throw new RangeError('copied text');
        w.raw(text);
      }
      break;
    case 'audio':
      if (m.payload.length === 0 || m.payload.length > MAX_AUDIO_PACKET) throw new RangeError('audio packet size');
      w.u64(m.timestampUs);
      w.raw(m.payload);
      break;
    case 'tiles':
      w.u32(m.epoch);
      w.u64(m.revision);
      if (m.rects.length > MAX_TILES) throw new RangeError('tile count');
      w.u16(m.rects.length);
      for (const rect of m.rects) {
        if (rect.w === 0 || rect.h === 0 || rect.w > MAX_TILE_SIDE || rect.h > MAX_TILE_SIDE) {
          throw new RangeError('tile size');
        }
        w.rect(rect);
      }
      w.u32(m.rawLen);
      w.raw(m.data);
      break;
    case 'cursor':
      w.u32(m.epoch);
      w.u16(m.x);
      w.u16(m.y);
      w.u8(m.shape);
      w.u16(m.hotX);
      w.u16(m.hotY);
      w.raw(m.image);
      break;
    case 'keyframe-request':
      w.u32(m.epoch);
      break;
    case 'server-stats':
      w.u64(m.serverTimeUs);
      w.u32(m.frames);
      w.u32(m.bytes);
      w.u32(m.encodeUsP50);
      w.u32(m.encodeUsP95);
      break;
    case 'client-stats':
      w.u32(m.presented);
      w.u32(m.decodeUsP50);
      w.bool(m.hardware);
      break;
    case 'time-ping':
      w.u64(m.clientTimeUs);
      break;
    case 'time-pong':
      w.u64(m.clientTimeUs);
      w.u64(m.serverTimeUs);
      break;
    case 'lease':
      w.u32(m.leaseEpoch);
      w.u8(m.holder);
      break;
    case 'pointer':
      w.u32(m.leaseEpoch);
      w.u8(m.kind);
      w.u8(m.button);
      w.u16(m.x);
      w.u16(m.y);
      break;
    case 'wheel':
      w.u32(m.leaseEpoch);
      w.u16(m.x);
      w.u16(m.y);
      w.i16(m.dx);
      w.i16(m.dy);
      break;
    case 'key':
      w.u32(m.leaseEpoch);
      w.u8(m.kind);
      w.u16(m.code);
      w.raw(utf8Encoder.encode(m.text));
      break;
    case 'clipboard':
      w.u32(m.leaseEpoch);
      w.raw(utf8Encoder.encode(m.text));
      break;
    case 'file-offer': {
      const name = utf8Encoder.encode(m.name);
      if (name.length === 0 || name.length > MAX_FILE_NAME) throw new RangeError('file name');
      w.u32(m.leaseEpoch);
      w.u32(m.file);
      w.bool(m.size !== null);
      w.u64(m.size ?? 0n);
      w.raw(name);
      break;
    }
    case 'file-link':
      if (m.ticket.length !== FILE_TICKET) throw new RangeError('file ticket');
      w.u32(m.leaseEpoch);
      w.u32(m.file);
      w.raw(m.ticket);
      break;
    case 'file-end':
      w.u32(m.leaseEpoch);
      w.u32(m.file);
      w.u64(m.size);
      break;
    case 'file-cancel':
      w.u32(m.leaseEpoch);
      w.u32(m.file);
      break;
    case 'file-data':
      if (m.data.length === 0 || m.data.length > FILE_CHUNK) throw new RangeError('file piece size');
      w.u32(m.leaseEpoch);
      w.u32(m.file);
      w.u64(m.offset);
      w.raw(m.data);
      break;
    case 'file-read':
      w.u32(m.leaseEpoch);
      w.u32(m.file);
      w.u64(m.offset);
      break;
    case 'file-gone':
      w.u32(m.leaseEpoch);
      w.u32(m.file);
      break;
    case 'file-request':
      w.u32(m.leaseEpoch);
      w.u32(m.request);
      w.bool(m.multiple);
      break;
    case 'file-request-end':
      w.u32(m.leaseEpoch);
      w.u32(m.request);
      break;
    case 'upload-ack':
      w.u32(m.leaseEpoch);
      w.u32(m.upload);
      w.u64(m.received);
      break;
    case 'upload-done':
      w.u32(m.leaseEpoch);
      w.u32(m.upload);
      w.bool(m.stored);
      break;
    case 'upload-start': {
      const name = utf8Encoder.encode(m.name);
      if (name.length === 0 || name.length > MAX_FILE_NAME) throw new RangeError('file name');
      w.u32(m.leaseEpoch);
      w.u32(m.upload);
      w.u64(m.size);
      w.raw(name);
      break;
    }
    case 'upload-data':
      if (m.data.length === 0 || m.data.length > UPLOAD_CHUNK) throw new RangeError('upload piece size');
      w.u32(m.leaseEpoch);
      w.u32(m.upload);
      w.u64(m.offset);
      w.raw(m.data);
      break;
    case 'file-choose':
      if (m.uploads.length > MAX_CHOSEN) throw new RangeError('upload list');
      w.u32(m.leaseEpoch);
      w.u32(m.request);
      for (const upload of m.uploads) w.u32(upload);
      break;
    case 'file-drop':
      if (m.uploads.length === 0 || m.uploads.length > MAX_CHOSEN) throw new RangeError('upload list');
      w.u32(m.leaseEpoch);
      w.u16(m.x);
      w.u16(m.y);
      for (const upload of m.uploads) w.u32(upload);
      break;
    case 'file-drag':
      if (m.types.length === 0 || m.types.length > MAX_CHOSEN) throw new RangeError('drag types');
      w.u32(m.leaseEpoch);
      w.u16(m.x);
      w.u16(m.y);
      w.u8(m.types.length);
      for (const t of m.types) {
        if (!isMediaType(t)) throw new RangeError('media type');
        w.u8(t.length);
        w.raw(utf8Encoder.encode(t));
      }
      break;
    case 'file-drag-end':
      w.u32(m.leaseEpoch);
      break;
    case 'record-start':
      w.u32(m.leaseEpoch);
      w.u32(m.recording);
      w.layout(m.layout);
      w.u8((m.camera ? 1 : 0) | (m.microphone ? 2 : 0) | (m.pageSound ? 4 : 0));
      break;
    case 'record-camera':
      if (m.jpeg.length === 0 || m.jpeg.length > MAX_RECORD_PICTURE) throw new RangeError('camera picture size');
      w.u32(m.leaseEpoch);
      w.u32(m.recording);
      w.u64(m.timestampUs);
      w.raw(m.jpeg);
      break;
    case 'record-microphone':
      if (m.payload.length === 0 || m.payload.length > MAX_AUDIO_PACKET) throw new RangeError('microphone packet size');
      w.u32(m.leaseEpoch);
      w.u32(m.recording);
      w.u64(m.timestampUs);
      w.raw(m.payload);
      break;
    case 'record-stop':
      w.u32(m.leaseEpoch);
      w.u32(m.recording);
      break;
    case 'record-layout':
      w.u32(m.leaseEpoch);
      w.u32(m.recording);
      w.layout(m.layout);
      break;
    case 'record-state':
      w.u32(m.leaseEpoch);
      w.u32(m.recording);
      w.u8(m.phase);
      w.u32(m.file);
      w.u32(m.seconds);
      w.u64(m.bytes);
      break;
  }
  return w.finish();
}

class Reader {
  private at = 0;
  private readonly view: DataView;

  constructor(private readonly bytes: Uint8Array) {
    this.view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  }

  get remaining(): number {
    return this.bytes.length - this.at;
  }

  private advance(n: number): number {
    if (this.remaining < n) throw new ProtocolError('truncated');
    const at = this.at;
    this.at += n;
    return at;
  }

  /** A view, not a copy: each message has its own buffer, and what is decoded from it is only read. */
  take(n: number): Uint8Array {
    const at = this.advance(n);
    return this.bytes.subarray(at, at + n);
  }
  rest(): Uint8Array {
    return this.take(this.remaining);
  }
  u8(): number {
    return this.view.getUint8(this.advance(1));
  }
  bool(): boolean {
    const v = this.u8();
    if (v > 1) throw new ProtocolError('invalid-value', 'flag');
    return v === 1;
  }
  layout(): CameraLayout {
    const x = this.u16();
    const y = this.u16();
    if (x > LAYOUT_UNITS || y > LAYOUT_UNITS) throw new ProtocolError('invalid-value', 'camera position');
    const side = this.u16();
    if (side < CAMERA_SIDE_MIN || side > CAMERA_SIDE_MAX) throw new ProtocolError('invalid-value', 'camera side');
    const radius = this.u8();
    if (radius > CAMERA_RADIUS_MAX) throw new ProtocolError('invalid-value', 'camera radius');
    const border = this.u8();
    if (border > 2) throw new ProtocolError('invalid-value', 'camera border');
    return { x, y, side, radius, border: border as CameraBorder };
  }

  u16(): number {
    return this.view.getUint16(this.advance(2), true);
  }
  i16(): number {
    return this.view.getInt16(this.advance(2), true);
  }
  u32(): number {
    return this.view.getUint32(this.advance(4), true);
  }
  u64(): bigint {
    return this.view.getBigUint64(this.advance(8), true);
  }
  rect(): Rect {
    const rect = { x: this.u16(), y: this.u16(), w: this.u16(), h: this.u16() };
    if (rect.w === 0 || rect.h === 0) throw new ProtocolError('invalid-value', 'rectangle');
    return rect;
  }
}

function closed<T extends number>(v: number, values: readonly T[], what: string): T {
  if (!(values as readonly number[]).includes(v)) throw new ProtocolError('invalid-value', what);
  return v as T;
}

function utf8(bytes: Uint8Array, max: number): string {
  if (bytes.length > max) throw new ProtocolError('too-large');
  try {
    return utf8Decoder.decode(bytes);
  } catch {
    throw new ProtocolError('invalid-value', 'utf-8');
  }
}

const MEDIA_TYPE = /^[a-z0-9!#$&^_.+-]+\/[a-z0-9!#$&^_.+-]+$/;

/** Empty (unknown), or a lowercase `type/subtype` of RFC 6838 restricted-name characters, at most MAX_DRAG_TYPE bytes. */
export function isMediaType(t: string): boolean {
  return t === '' || (t.length <= MAX_DRAG_TYPE && MEDIA_TYPE.test(t));
}

/** A list of upload ids, at least `min` and at most MAX_CHOSEN. */
function uploadIds(bytes: Uint8Array, min: number): number[] {
  const count = bytes.length / 4;
  if (bytes.length % 4 !== 0 || count < min || count > MAX_CHOSEN) throw new ProtocolError('invalid-value', 'upload list');
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return Array.from({ length: count }, (_, i) => view.getUint32(4 * i, true));
}

function body(r: Reader, kind: number): Message {
  switch (kind) {
    case KIND.hello: {
      const version = r.u16();
      const codecs = r.u8();
      // One version on the wire: a client of another one is refused before it gets a configuration.
      if (version !== VERSION) throw new ProtocolError('invalid-value', 'version');
      if ((codecs & ~KNOWN_CODECS) !== 0) throw new ProtocolError('invalid-value', 'codec set');
      return { type: 'hello', version, codecs };
    }
    case KIND.config: {
      const version = r.u16();
      const epoch = r.u32();
      const width = r.u16();
      const height = r.u16();
      const codec = closed(r.u8(), Object.values(Codec), 'codec');
      const maxFps = r.u8();
      const sized = (side: number) => side >= 1 && side <= MAX_OUTPUT_SIDE;
      if (version !== VERSION || !sized(width) || !sized(height) || maxFps === 0) {
        throw new ProtocolError('invalid-value', 'configuration');
      }
      return { type: 'config', version, epoch, width, height, codec, maxFps };
    }
    case KIND.video:
      return {
        type: 'video',
        epoch: r.u32(),
        revision: r.u64(),
        rect: r.rect(),
        keyframe: r.bool(),
        timestampUs: r.u64(),
        payload: r.rest(),
      };
    case KIND.copied:
      return { type: 'copied', leaseEpoch: r.u32(), text: utf8(r.rest(), MAX_COPIED) };
    case KIND.audio: {
      const timestampUs = r.u64();
      const payload = r.rest();
      if (payload.length === 0 || payload.length > MAX_AUDIO_PACKET) {
        throw new ProtocolError('invalid-value', 'audio packet size');
      }
      return { type: 'audio', timestampUs, payload };
    }
    case KIND.tiles: {
      const epoch = r.u32();
      const revision = r.u64();
      const count = r.u16();
      if (count === 0 || count > MAX_TILES) throw new ProtocolError('invalid-value', 'tile count');
      const rects: Rect[] = [];
      let longest = 0;
      for (let i = 0; i < count; i++) {
        const rect = r.rect();
        if (rect.w > MAX_TILE_SIDE || rect.h > MAX_TILE_SIDE) {
          throw new ProtocolError('invalid-value', 'tile size');
        }
        rects.push(rect);
        longest += 1 + 3 * rect.w * rect.h;
      }
      // Between every rectangle solid and every one RGB (see protocol/src/records.rs).
      const rawLen = r.u32();
      if (rawLen < 4 * count || rawLen > Math.min(longest, MAX_TILES_RAW)) {
        throw new ProtocolError('invalid-value', 'tile records length');
      }
      return { type: 'tiles', epoch, revision, rects, rawLen, data: r.rest() };
    }
    case KIND.cursor: {
      const cursor = {
        type: 'cursor',
        epoch: r.u32(),
        x: r.u16(),
        y: r.u16(),
        shape: r.u8(),
        hotX: r.u16(),
        hotY: r.u16(),
        image: r.rest(),
      } as const;
      if (cursor.image.length > MAX_CURSOR_IMAGE) throw new ProtocolError('too-large', 'cursor image');
      // A shape the protocol numbers (0 hidden, 1-34 named, 255 custom), and an image exactly for the custom one.
      if (!(cursor.shape <= 34 || cursor.shape === 255) || (cursor.image.length === 0) !== (cursor.shape !== 255)) {
        throw new ProtocolError('invalid-value', 'cursor shape');
      }
      return cursor;
    }
    case KIND['keyframe-request']:
      return { type: 'keyframe-request', epoch: r.u32() };
    case KIND['server-stats']:
      return {
        type: 'server-stats',
        serverTimeUs: r.u64(),
        frames: r.u32(),
        bytes: r.u32(),
        encodeUsP50: r.u32(),
        encodeUsP95: r.u32(),
      };
    case KIND['client-stats']:
      return { type: 'client-stats', presented: r.u32(), decodeUsP50: r.u32(), hardware: r.bool() };
    case KIND['time-ping']:
      return { type: 'time-ping', clientTimeUs: r.u64() };
    case KIND['time-pong']:
      return { type: 'time-pong', clientTimeUs: r.u64(), serverTimeUs: r.u64() };
    case KIND.lease:
      return { type: 'lease', leaseEpoch: r.u32(), holder: closed(r.u8(), Object.values(LeaseHolder), 'lease holder') };
    case KIND.pointer:
      return {
        type: 'pointer',
        leaseEpoch: r.u32(),
        kind: closed(r.u8(), Object.values(PointerKind), 'pointer kind'),
        // Buttons follow the DOM `MouseEvent.button` numbering: 0 primary, 1 middle, 2 secondary.
        button: closed(r.u8(), [0, 1, 2], 'button'),
        x: r.u16(),
        y: r.u16(),
      };
    case KIND.wheel:
      return { type: 'wheel', leaseEpoch: r.u32(), x: r.u16(), y: r.u16(), dx: r.i16(), dy: r.i16() };
    case KIND.key: {
      const leaseEpoch = r.u32();
      const kind = closed(r.u8(), Object.values(KeyKind), 'key kind');
      const code = r.u16();
      const text = utf8(r.rest(), MAX_TEXT);
      if ((kind === KeyKind.Text) === (text.length === 0) || (kind === KeyKind.Text) !== (code === 0)) {
        throw new ProtocolError('invalid-value', 'key');
      }
      return { type: 'key', leaseEpoch, kind, code, text };
    }
    case KIND.clipboard:
      return { type: 'clipboard', leaseEpoch: r.u32(), text: utf8(r.rest(), MAX_CLIPBOARD) };
    case KIND['file-offer']: {
      const leaseEpoch = r.u32();
      const file = r.u32();
      const known = r.bool();
      const size = r.u64();
      if (!known && size !== 0n) throw new ProtocolError('invalid-value', 'file size');
      const name = utf8(r.rest(), MAX_FILE_NAME);
      if (name.length === 0) throw new ProtocolError('invalid-value', 'file name');
      return { type: 'file-offer', leaseEpoch, file, size: known ? size : null, name };
    }
    case KIND['file-link']:
      return { type: 'file-link', leaseEpoch: r.u32(), file: r.u32(), ticket: r.take(FILE_TICKET) };
    case KIND['file-end']:
      return { type: 'file-end', leaseEpoch: r.u32(), file: r.u32(), size: r.u64() };
    case KIND['file-cancel']:
      return { type: 'file-cancel', leaseEpoch: r.u32(), file: r.u32() };
    case KIND['file-data']: {
      const leaseEpoch = r.u32();
      const file = r.u32();
      const offset = r.u64();
      const data = r.rest();
      if (data.length === 0 || data.length > FILE_CHUNK) throw new ProtocolError('invalid-value', 'file piece size');
      return { type: 'file-data', leaseEpoch, file, offset, data };
    }
    case KIND['file-read']:
      return { type: 'file-read', leaseEpoch: r.u32(), file: r.u32(), offset: r.u64() };
    case KIND['file-gone']:
      return { type: 'file-gone', leaseEpoch: r.u32(), file: r.u32() };
    case KIND['file-request']:
      return { type: 'file-request', leaseEpoch: r.u32(), request: r.u32(), multiple: r.bool() };
    case KIND['file-request-end']:
      return { type: 'file-request-end', leaseEpoch: r.u32(), request: r.u32() };
    case KIND['upload-ack']:
      return { type: 'upload-ack', leaseEpoch: r.u32(), upload: r.u32(), received: r.u64() };
    case KIND['upload-done']:
      return { type: 'upload-done', leaseEpoch: r.u32(), upload: r.u32(), stored: r.bool() };
    case KIND['upload-start']: {
      const leaseEpoch = r.u32();
      const upload = r.u32();
      const size = r.u64();
      const name = utf8(r.rest(), MAX_FILE_NAME);
      if (name.length === 0) throw new ProtocolError('invalid-value', 'file name');
      return { type: 'upload-start', leaseEpoch, upload, size, name };
    }
    case KIND['upload-data']: {
      const leaseEpoch = r.u32();
      const upload = r.u32();
      const offset = r.u64();
      const data = r.rest();
      if (data.length === 0 || data.length > UPLOAD_CHUNK) throw new ProtocolError('invalid-value', 'upload piece size');
      return { type: 'upload-data', leaseEpoch, upload, offset, data };
    }
    case KIND['file-choose']: {
      const leaseEpoch = r.u32();
      const request = r.u32();
      return { type: 'file-choose', leaseEpoch, request, uploads: uploadIds(r.rest(), 0) };
    }
    case KIND['file-drop']: {
      const leaseEpoch = r.u32();
      const x = r.u16();
      const y = r.u16();
      return { type: 'file-drop', leaseEpoch, x, y, uploads: uploadIds(r.rest(), 1) };
    }
    case KIND['file-drag']: {
      const leaseEpoch = r.u32();
      const x = r.u16();
      const y = r.u16();
      const count = r.u8();
      if (count === 0 || count > MAX_CHOSEN) throw new ProtocolError('invalid-value', 'drag types');
      const types: string[] = [];
      for (let i = 0; i < count; i++) {
        const t = utf8(r.take(r.u8()), MAX_DRAG_TYPE);
        if (!isMediaType(t)) throw new ProtocolError('invalid-value', 'media type');
        types.push(t);
      }
      return { type: 'file-drag', leaseEpoch, x, y, types };
    }
    case KIND['file-drag-end']:
      return { type: 'file-drag-end', leaseEpoch: r.u32() };
    case KIND['record-start']: {
      const leaseEpoch = r.u32();
      const recording = r.u32();
      const layout = r.layout();
      const flags = r.u8();
      if (flags & ~0b111) throw new ProtocolError('invalid-value', 'recording flags');
      return {
        type: 'record-start',
        leaseEpoch,
        recording,
        layout,
        camera: (flags & 1) !== 0,
        microphone: (flags & 2) !== 0,
        pageSound: (flags & 4) !== 0,
      };
    }
    case KIND['record-camera']: {
      const leaseEpoch = r.u32();
      const recording = r.u32();
      const timestampUs = r.u64();
      const jpeg = r.rest();
      if (jpeg.length === 0 || jpeg.length > MAX_RECORD_PICTURE) throw new ProtocolError('invalid-value', 'camera picture size');
      return { type: 'record-camera', leaseEpoch, recording, timestampUs, jpeg };
    }
    case KIND['record-microphone']: {
      const leaseEpoch = r.u32();
      const recording = r.u32();
      const timestampUs = r.u64();
      const payload = r.rest();
      if (payload.length === 0 || payload.length > MAX_AUDIO_PACKET) {
        throw new ProtocolError('invalid-value', 'microphone packet size');
      }
      return { type: 'record-microphone', leaseEpoch, recording, timestampUs, payload };
    }
    case KIND['record-stop']:
      return { type: 'record-stop', leaseEpoch: r.u32(), recording: r.u32() };
    case KIND['record-layout']: {
      const leaseEpoch = r.u32();
      const recording = r.u32();
      return { type: 'record-layout', leaseEpoch, recording, layout: r.layout() };
    }
    case KIND['record-state']: {
      const leaseEpoch = r.u32();
      const recording = r.u32();
      const phase = r.u8();
      if (phase > 2) throw new ProtocolError('invalid-value', 'recording phase');
      return { type: 'record-state', leaseEpoch, recording, phase: phase as RecordPhase, file: r.u32(), seconds: r.u32(), bytes: r.u64() };
    }
    default:
      throw new ProtocolError('unknown-kind', `0x${kind.toString(16).padStart(2, '0')}`);
  }
}

/** Decodes one message sent in `dir`, refusing anything outside the closed set. */
export function decode(bytes: Uint8Array, dir: Direction): Message {
  const limit = dir === 'client-to-server' ? MAX_CLIENT_MESSAGE : MAX_SERVER_MESSAGE;
  if (bytes.length > limit) throw new ProtocolError('too-large');
  if (bytes.length === 0) throw new ProtocolError('empty');
  const r = new Reader(bytes);
  const message = body(r, r.u8());
  if (r.remaining !== 0) throw new ProtocolError('trailing-bytes');
  if (direction(message) !== dir) throw new ProtocolError('invalid-value', 'direction');
  return message;
}
