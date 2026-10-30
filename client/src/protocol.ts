// The easi.run wire protocol, version `VERSION`: the browser's half of protocol/src/lib.rs. It encodes the messages a
// client sends and decodes those a server sends a browser; the download pieces the admin keeps from the browser
// (`FileData`, `FileRead`, `FileCancel`) are left out.
//
// One message is one WebSocket binary frame: a one-byte kind followed by fixed little-endian fields. The set is
// closed, and decoding refuses an unknown kind, a short or over-long body, an out-of-range value, or invalid UTF-8,
// with the same error (and in the same order) as the Rust decoder; a client's message, or a download piece, coming
// from the server is refused as an unknown kind.

export const VERSION = 5;
export const MAX_CLIPBOARD = 64 * 1024;
export const MAX_SERVER_MESSAGE = 2 * 1024 * 1024;
export const MAX_TEXT = 1024;
/** Largest file name, in bytes. */
export const MAX_FILE_NAME = 255;
/** Length of a download ticket, in bytes. */
const FILE_TICKET = 32;
/** Largest piece of an upload, the bytes a client may send ahead of the last acknowledgement, and most uploads named. */
export const UPLOAD_CHUNK = 60 * 1024;
export const UPLOAD_WINDOW = 256 * 1024;
export const MAX_CHOSEN = 64;
/** Largest media type a `file-drag` names, in bytes. */
const MAX_DRAG_TYPE = 127;
/** Largest address a `udp-offer` carries, in bytes. */
const MAX_UDP_URL = 512;
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
}
export const LeaseHolder = { Agent: 0, You: 1, Other: 2 } as const;
export type LeaseHolder = (typeof LeaseHolder)[keyof typeof LeaseHolder];

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** What a client sends. */
export type ClientMessage =
  | { type: 'hello'; version: number; codecs: number }
  | { type: 'upload-start'; leaseEpoch: number; upload: number; size: bigint; name: string }
  | { type: 'upload-data'; leaseEpoch: number; upload: number; offset: bigint; data: Uint8Array }
  | { type: 'file-choose'; leaseEpoch: number; request: number; uploads: number[] }
  | { type: 'file-drop'; leaseEpoch: number; x: number; y: number; uploads: number[] }
  | { type: 'file-drag'; leaseEpoch: number; x: number; y: number; types: string[] }
  | { type: 'file-drag-end'; leaseEpoch: number }
  | { type: 'keyframe-request'; epoch: number }
  | { type: 'media-ack'; revision: bigint }
  | { type: 'udp-request' }
  | { type: 'client-stats'; presented: number; decodeUsP50: number; hardware: boolean }
  | { type: 'time-ping'; clientTimeUs: bigint }
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
  | { type: 'record-layout'; leaseEpoch: number; recording: number; layout: CameraLayout };

/** What a server sends a browser. */
export type ServerMessage =
  | { type: 'config'; version: number; epoch: number; width: number; height: number; codec: Codec; maxFps: number }
  | { type: 'video'; epoch: number; revision: bigint; rect: Rect; keyframe: boolean; timestampUs: bigint; payload: Uint8Array }
  | { type: 'audio'; timestampUs: bigint; payload: Uint8Array }
  | { type: 'copied'; leaseEpoch: number; text: string }
  | { type: 'file-offer'; leaseEpoch: number; file: number; size: bigint | null; name: string }
  | { type: 'file-link'; leaseEpoch: number; file: number; ticket: Uint8Array }
  | { type: 'file-end'; leaseEpoch: number; file: number; size: bigint }
  | { type: 'file-gone'; leaseEpoch: number; file: number }
  | { type: 'file-request'; leaseEpoch: number; request: number; multiple: boolean }
  | { type: 'file-request-end'; leaseEpoch: number; request: number }
  | { type: 'upload-ack'; leaseEpoch: number; upload: number; received: bigint }
  | { type: 'upload-done'; leaseEpoch: number; upload: number; stored: boolean }
  | { type: 'tiles'; epoch: number; revision: bigint; rects: Rect[]; rawLen: number; data: Uint8Array }
  | { type: 'cursor'; epoch: number; revision: bigint; x: number; y: number; shape: number; hotX: number; hotY: number; image: Uint8Array }
  | { type: 'server-stats'; serverTimeUs: bigint; frames: number; bytes: number; encodeUsP50: number; encodeUsP95: number }
  | { type: 'time-pong'; clientTimeUs: bigint; serverTimeUs: bigint }
  | { type: 'lease'; leaseEpoch: number; holder: LeaseHolder }
  | { type: 'udp-offer'; generation: number; certSha256: Uint8Array; url: string }
  | { type: 'media-moved'; generation: number }
  | { type: 'media-fallback'; generation: number }
  | { type: 'record-state'; leaseEpoch: number; recording: number; phase: RecordPhase; file: number; seconds: number; bytes: bigint }
  | { type: 'agent-pointer'; x: number; y: number };

export type Message = ClientMessage | ServerMessage;

export type MessageOf<T extends Message['type']> = Extract<Message, { type: T }>;

/** Each message's kind byte, its first. */
export const KIND = {
  hello: 0x01,
  config: 0x02,
  video: 0x03,
  tiles: 0x04,
  cursor: 0x05,
  'keyframe-request': 0x06,
  'media-ack': 0x29,
  'udp-request': 0x2a,
  'udp-offer': 0x2b,
  'media-moved': 0x2c,
  'media-fallback': 0x2d,
  'agent-pointer': 0x2e,
  'server-stats': 0x07,
  'client-stats': 0x08,
  'time-ping': 0x09,
  'time-pong': 0x0a,
  lease: 0x0b,
  audio: 0x0c,
  copied: 0x0d,
  'file-offer': 0x0e,
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
  'upload-start': 0x15,
  'upload-data': 0x16,
  'file-choose': 0x17,
  'file-drop': 0x18,
  'file-drag': 0x21,
  'file-drag-end': 0x22,
  'record-start': 0x23,
  'record-camera': 0x24,
  'record-microphone': 0x25,
  'record-stop': 0x26,
  'record-state': 0x27,
  'record-layout': 0x28,
} as const satisfies Record<Message['type'], number>;

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

/** A little-endian message builder; each write returns it, so writes chain. */
export class Writer {
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
  u8(v: number): this {
    const at = this.reserve(1);
    this.view.setUint8(at, this.int(v, 0, 0xff));
    return this;
  }
  bool(v: boolean): this {
    this.u8(v ? 1 : 0);
    return this;
  }
  layout(l: CameraLayout): this {
    this.u16(l.x);
    this.u16(l.y);
    this.u16(l.side);
    this.u8(l.radius);
    return this;
  }

  u16(v: number): this {
    const at = this.reserve(2);
    this.view.setUint16(at, this.int(v, 0, 0xffff), true);
    return this;
  }
  i16(v: number): this {
    const at = this.reserve(2);
    this.view.setInt16(at, this.int(v, -0x8000, 0x7fff), true);
    return this;
  }
  u32(v: number): this {
    const at = this.reserve(4);
    this.view.setUint32(at, this.int(v, 0, 0xffff_ffff), true);
    return this;
  }
  u64(v: bigint): this {
    if (v < 0n || v > 0xffff_ffff_ffff_ffffn) throw new RangeError(`value ${v} outside u64`);
    const at = this.reserve(8);
    this.view.setBigUint64(at, v, true);
    return this;
  }
  raw(b: Uint8Array): this {
    const at = this.reserve(b.length);
    this.bytes.set(b, at);
    return this;
  }
  finish(): Uint8Array<ArrayBuffer> {
    return this.bytes.slice(0, this.len);
  }
}

/** Encodes one message. Integer fields outside their wire width throw instead of wrapping. */
export function encode(m: ClientMessage): Uint8Array<ArrayBuffer> {
  const w = new Writer();
  w.u8(KIND[m.type]);
  switch (m.type) {
    case 'hello':
      w.u16(m.version);
      w.u8(m.codecs);
      break;
    case 'keyframe-request':
      w.u32(m.epoch);
      break;
    case 'media-ack':
      w.u64(m.revision);
      break;
    case 'udp-request':
      break;
    case 'client-stats':
      w.u32(m.presented);
      w.u32(m.decodeUsP50);
      w.bool(m.hardware);
      break;
    case 'time-ping':
      w.u64(m.clientTimeUs);
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
  u16(): number {
    return this.view.getUint16(this.advance(2), true);
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

function body(r: Reader, kind: number): ServerMessage {
  switch (kind) {
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
        revision: r.u64(),
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
    case KIND['server-stats']:
      return {
        type: 'server-stats',
        serverTimeUs: r.u64(),
        frames: r.u32(),
        bytes: r.u32(),
        encodeUsP50: r.u32(),
        encodeUsP95: r.u32(),
      };
    case KIND['time-pong']:
      return { type: 'time-pong', clientTimeUs: r.u64(), serverTimeUs: r.u64() };
    case KIND.lease:
      return { type: 'lease', leaseEpoch: r.u32(), holder: closed(r.u8(), Object.values(LeaseHolder), 'lease holder') };
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
    case KIND['udp-offer']: {
      const generation = r.u32();
      const certSha256 = r.take(32);
      const url = utf8(r.rest(), MAX_UDP_URL);
      if (!/^https:\/\/[\x21-\x7e]*$/.test(url)) throw new ProtocolError('invalid-value', 'udp address');
      return { type: 'udp-offer', generation, certSha256, url };
    }
    case KIND['media-moved']:
      return { type: 'media-moved', generation: r.u32() };
    case KIND['media-fallback']:
      return { type: 'media-fallback', generation: r.u32() };
    case KIND['agent-pointer']:
      return { type: 'agent-pointer', x: r.u16(), y: r.u16() };
    case KIND['file-end']:
      return { type: 'file-end', leaseEpoch: r.u32(), file: r.u32(), size: r.u64() };
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
    case KIND['record-state']: {
      const leaseEpoch = r.u32();
      const recording = r.u32();
      const phase = closed(r.u8(), Object.values(RecordPhase), 'recording phase');
      return { type: 'record-state', leaseEpoch, recording, phase, file: r.u32(), seconds: r.u32(), bytes: r.u64() };
    }
    default:
      throw new ProtocolError('unknown-kind', `0x${kind.toString(16).padStart(2, '0')}`);
  }
}

/** Decodes one message the server sent, refusing anything outside the closed set. */
export function decode(bytes: Uint8Array): ServerMessage {
  if (bytes.length > MAX_SERVER_MESSAGE) throw new ProtocolError('too-large');
  if (bytes.length === 0) throw new ProtocolError('empty');
  const r = new Reader(bytes);
  const message = body(r, r.u8());
  if (r.remaining !== 0) throw new ProtocolError('trailing-bytes');
  return message;
}
