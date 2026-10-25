// The preview's stand-in for a session's viewer stream: just enough WebSocket (RFC 6455) to talk to the viewer, the few
// server messages of protocol.ts it needs, and a PNG reader, so an example screen goes out as lossless tiles. Dev server
// only: nothing here is part of the client's build.

import { createHash } from 'node:crypto';
import type { IncomingMessage } from 'node:http';
import type { Duplex } from 'node:stream';
import { deflateRawSync, inflateSync } from 'node:zlib';

/** One WebSocket connection, server side: binary frames out, the client's binary frames in. */
export class Socket {
  private pending = Buffer.alloc(0);
  closed = false;
  /** Each binary message the client sends. */
  onMessage: (data: Buffer) => void = () => {};
  /** The connection ended, from either side. */
  onClose: () => void = () => {};

  private readonly raw: Duplex;

  constructor(raw: Duplex) {
    this.raw = raw;
    raw.on('data', (chunk: Buffer) => this.read(chunk));
    raw.on('close', () => this.gone());
    raw.on('error', () => this.gone());
  }

  /** Answers the upgrade request: false (and the socket ended) when it is not a WebSocket handshake. */
  static accept(req: IncomingMessage, raw: Duplex): boolean {
    const key = req.headers['sec-websocket-key'];
    if (typeof key !== 'string' || req.headers.upgrade?.toLowerCase() !== 'websocket') {
      raw.end('HTTP/1.1 400 Bad Request\r\n\r\n');
      return false;
    }
    const accept = createHash('sha1').update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64');
    raw.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    return true;
  }

  send(data: Uint8Array): void {
    this.frame(0x2, Buffer.from(data.buffer, data.byteOffset, data.byteLength));
  }

  /** Closes with a close frame (`code`), as a server ending the stream on purpose does. */
  close(code: number, reason = ''): void {
    if (this.closed) return;
    const body = Buffer.alloc(2 + Buffer.byteLength(reason));
    body.writeUInt16BE(code, 0);
    body.write(reason, 2);
    this.frame(0x8, body);
    this.raw.end();
    this.gone();
  }

  /** Drops the connection without a word, as a network that fails does (the browser sees 1006). */
  drop(): void {
    this.raw.destroy();
    this.gone();
  }

  private gone(): void {
    if (this.closed) return;
    this.closed = true;
    this.onClose();
  }

  private frame(opcode: number, payload: Buffer): void {
    if (this.closed) return;
    const n = payload.length;
    const head = n < 126 ? Buffer.alloc(2) : n < 65536 ? Buffer.alloc(4) : Buffer.alloc(10);
    head[0] = 0x80 | opcode;
    if (n < 126) head[1] = n;
    else if (n < 65536) {
      head[1] = 126;
      head.writeUInt16BE(n, 2);
    } else {
      head[1] = 127;
      head.writeBigUInt64BE(BigInt(n), 2);
    }
    this.raw.write(Buffer.concat([head, payload]));
  }

  private read(chunk: Buffer): void {
    this.pending = Buffer.concat([this.pending, chunk]);
    for (;;) {
      const b = this.pending;
      if (b.length < 2) return;
      const opcode = b[0]! & 0x0f;
      let len = b[1]! & 0x7f;
      let at = 2;
      if (len === 126) {
        if (b.length < 4) return;
        len = b.readUInt16BE(2);
        at = 4;
      } else if (len === 127) {
        if (b.length < 10) return;
        len = Number(b.readBigUInt64BE(2));
        at = 10;
      }
      const masked = (b[1]! & 0x80) !== 0;
      const mask = masked ? b.subarray(at, at + 4) : null;
      if (masked) at += 4;
      if (b.length < at + len) return;
      const payload = Buffer.from(b.subarray(at, at + len));
      if (mask) for (let i = 0; i < payload.length; i++) payload[i]! ^= mask[i % 4]!;
      this.pending = b.subarray(at + len);
      if (opcode === 0x2) this.onMessage(payload);
      else if (opcode === 0x9) this.frame(0xa, payload);
      else if (opcode === 0x8) return this.close(1000);
    }
  }
}

/** A little-endian message builder, one kind byte first. */
class Out {
  private parts: Buffer[] = [];
  constructor(kind: number) {
    this.u8(kind);
  }
  u8(v: number): this {
    this.parts.push(Buffer.from([v]));
    return this;
  }
  u16(v: number): this {
    const b = Buffer.alloc(2);
    b.writeUInt16LE(v);
    this.parts.push(b);
    return this;
  }
  u32(v: number): this {
    const b = Buffer.alloc(4);
    b.writeUInt32LE(v >>> 0);
    this.parts.push(b);
    return this;
  }
  u64(v: bigint): this {
    const b = Buffer.alloc(8);
    b.writeBigUInt64LE(v);
    this.parts.push(b);
    return this;
  }
  raw(b: Uint8Array): this {
    this.parts.push(Buffer.from(b));
    return this;
  }
  done(): Buffer {
    return Buffer.concat(this.parts);
  }
}

export const Codec = { None: 0, Av1: 1, H264: 2 } as const;
export const Holder = { Agent: 0, You: 1, Other: 2 } as const;
export const Phase = { Recording: 0, Ended: 1 } as const;

/** The server messages the preview sends (see client/src/protocol.ts for the layout of each). */
export const message = {
  config: (epoch: number, width: number, height: number, codec: number) =>
    new Out(0x02).u16(1).u32(epoch).u16(width).u16(height).u8(codec).u8(30).done(),
  cursor: (epoch: number, x: number, y: number) => new Out(0x05).u32(epoch).u16(x).u16(y).u8(1).u16(0).u16(0).done(),
  serverStats: (now: bigint, frames: number, bytes: number, p50: number, p95: number) =>
    new Out(0x07).u64(now).u32(frames).u32(bytes).u32(p50).u32(p95).done(),
  pong: (client: bigint, server: bigint) => new Out(0x0a).u64(client).u64(server).done(),
  lease: (epoch: number, holder: number) => new Out(0x0b).u32(epoch).u8(holder).done(),
  copied: (epoch: number, text: string) => new Out(0x0d).u32(epoch).raw(Buffer.from(text)).done(),
  fileOffer: (epoch: number, file: number, name: string) => new Out(0x0e).u32(epoch).u32(file).u8(0).u64(0n).raw(Buffer.from(name)).done(),
  fileLink: (epoch: number, file: number, ticket: Uint8Array) => new Out(0x20).u32(epoch).u32(file).raw(ticket).done(),
  fileEnd: (epoch: number, file: number, size: number) => new Out(0x1f).u32(epoch).u32(file).u64(BigInt(size)).done(),
  fileRequest: (epoch: number, request: number) => new Out(0x1a).u32(epoch).u32(request).u8(1).done(),
  recordState: (epoch: number, recording: number, phase: number, file: number, seconds: number, bytes: number) =>
    new Out(0x27).u32(epoch).u32(recording).u8(phase).u32(file).u32(seconds).u64(BigInt(bytes)).done(),
};

/** The client messages the preview answers, decoded as far as it needs them. */
export type Incoming =
  | { type: 'hello'; codecs: number }
  | { type: 'ping'; client: bigint }
  | { type: 'record-start'; recording: number }
  | { type: 'record-stop'; recording: number }
  | { type: 'other' };

export function incoming(b: Buffer): Incoming {
  switch (b[0]) {
    case 0x01:
      return { type: 'hello', codecs: b[3] ?? 0 };
    case 0x09:
      return { type: 'ping', client: b.readBigUInt64LE(1) };
    case 0x23:
      return { type: 'record-start', recording: b.readUInt32LE(5) };
    case 0x26:
      return { type: 'record-stop', recording: b.readUInt32LE(5) };
    default:
      return { type: 'other' };
  }
}

/** An RGB picture, top row first. */
export interface Picture {
  width: number;
  height: number;
  rgb: Buffer;
}

/** Reads an 8-bit, non-interlaced RGB or RGBA PNG (what a Playwright screenshot is) to RGB. */
export function readPng(file: Buffer): Picture {
  let width = 0;
  let height = 0;
  let type = 0;
  const idat: Buffer[] = [];
  for (let at = 8; at < file.length; ) {
    const len = file.readUInt32BE(at);
    const name = file.toString('latin1', at + 4, at + 8);
    const data = file.subarray(at + 8, at + 8 + len);
    if (name === 'IHDR') {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      type = data[9]!;
      if (data[8] !== 8 || (type !== 2 && type !== 6) || data[12] !== 0) throw new Error('an unsupported PNG');
    } else if (name === 'IDAT') idat.push(data);
    at += 12 + len;
  }
  const channels = type === 6 ? 4 : 3;
  const stride = width * channels;
  const raw = inflateSync(Buffer.concat(idat));
  const rows = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)]!;
    const src = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const row = rows.subarray(y * stride, (y + 1) * stride);
    const up = y > 0 ? rows.subarray((y - 1) * stride, y * stride) : null;
    for (let i = 0; i < stride; i++) {
      const a = i >= channels ? row[i - channels]! : 0;
      const b = up ? up[i]! : 0;
      const c = up && i >= channels ? up[i - channels]! : 0;
      let p = 0;
      if (filter === 1) p = a;
      else if (filter === 2) p = b;
      else if (filter === 3) p = (a + b) >> 1;
      else if (filter === 4) {
        const pa = Math.abs(b - c);
        const pb = Math.abs(a - c);
        const pc = Math.abs(a + b - 2 * c);
        p = pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      row[i] = (src[i]! + p) & 0xff;
    }
  }
  const rgb = Buffer.alloc(width * height * 3);
  for (let p = 0; p < width * height; p++) rows.copy(rgb, p * 3, p * channels, p * channels + 3);
  return { width, height, rgb };
}

/** The whole picture as tiles messages, one per band of 256 rows, every tile an RGB record (protocol/src/records.rs). */
export function tiles(picture: Picture, epoch: number): Buffer[] {
  const SIDE = 256;
  const out: Buffer[] = [];
  for (let y = 0; y < picture.height; y += SIDE) {
    const h = Math.min(SIDE, picture.height - y);
    const rects: { x: number; w: number }[] = [];
    for (let x = 0; x < picture.width; x += SIDE) rects.push({ x, w: Math.min(SIDE, picture.width - x) });
    const records: Buffer[] = [];
    for (const { x, w } of rects) {
      records.push(Buffer.from([0]));
      for (let row = y; row < y + h; row++) {
        const start = (row * picture.width + x) * 3;
        records.push(picture.rgb.subarray(start, start + w * 3));
      }
    }
    const raw = Buffer.concat(records);
    const m = new Out(0x04).u32(epoch).u64(1n).u16(rects.length);
    for (const { x, w } of rects) m.u16(x).u16(y).u16(w).u16(h);
    out.push(m.u32(raw.length).raw(deflateRawSync(raw)).done());
  }
  return out;
}
