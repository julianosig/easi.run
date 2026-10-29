// The direct path: a WebTransport session beside the WebSocket that carries the media where UDP reaches the server
// (see protocol/src/lib.rs). The viewer asks for it whenever it connects and again every RETRY_MS while it has none;
// the WebSocket carries everything meanwhile, so a network that blocks UDP only means the media stay on it.

import { awaitAtMost } from './deadline.ts';
import { decode, encode, MAX_SERVER_MESSAGE, type ClientMessage, type MessageOf, type ServerMessage } from './protocol.ts';

/** How long a session may take to open before the viewer gives up on it. */
export const OPEN_TIMEOUT_MS = 3000;
/** How often the viewer asks again while it has no session. */
export const RETRY_MS = 30_000;
/** What may wait on the session before `media-moved` comes: past this the viewer gives the session up. */
export const MAX_BUFFERED = 2 * 1024 * 1024;

const MEDIA = new Set(['config', 'video', 'tiles', 'cursor']);

/**
 * Splits the session's media stream into messages: each a little-endian u32 length, then the display link's relay
 * tag (0) and the message, at most a server message long.
 */
export class Frames {
  private buf = new Uint8Array(64 * 1024);
  private start = 0;
  private end = 0;

  /** The messages complete once `chunk` arrived; throws on a frame no server sends. */
  push(chunk: Uint8Array): Uint8Array[] {
    this.append(chunk);
    const out: Uint8Array[] = [];
    while (this.end - this.start >= 4) {
      const at = this.start;
      const len = new DataView(this.buf.buffer, at, 4).getUint32(0, true);
      if (len === 0 || len > MAX_SERVER_MESSAGE + 1) throw new Error(`a frame of ${len} bytes on the media stream`);
      if (this.end - at > 4 && this.buf[at + 4] !== 0) throw new Error('a frame that is not a relayed message on the media stream');
      if (this.end - at - 4 < len) break;
      out.push(this.buf.slice(at + 5, at + 4 + len));
      this.start = at + 4 + len;
    }
    return out;
  }

  /** Appends `chunk` after what is pending, moving that to the front or growing the buffer when it must. */
  private append(chunk: Uint8Array): void {
    const pending = this.end - this.start;
    if (this.end + chunk.length > this.buf.length) {
      const size = Math.max(this.buf.length, pending + chunk.length);
      const grown = size > this.buf.length ? new Uint8Array(Math.max(size, this.buf.length * 2)) : this.buf;
      // (Within one buffer, `set` copies as if through a temporary, so an overlapping move is safe.)
      grown.set(this.buf.subarray(this.start, this.end), 0);
      this.buf = grown;
      this.start = 0;
      this.end = pending;
    }
    this.buf.set(chunk, this.end);
    this.end += chunk.length;
  }
}

/** One client message framed for the session's stream back to the server. */
export function frame(m: ClientMessage): Uint8Array<ArrayBuffer> {
  const message = encode(m);
  const out = new Uint8Array(5 + message.length);
  new DataView(out.buffer).setUint32(0, message.length + 1, true);
  out.set(message, 5);
  return out;
}

/**
 * Where the media come from, and what the viewer takes: media of the session wait (unacknowledged) until the WebSocket
 * says they moved, behind everything it carried; after a fallback, media wait for the new epoch's configuration.
 */
export class Handover {
  /** The offer whose session is open, while one is. */
  generation: number | null = null;
  /** Whether the media come over that session. */
  moved = false;
  private waiting: ServerMessage[] = [];
  private waitingBytes = 0;
  private awaitingConfig = false;

  /** The session of offer `generation` is being opened: what comes over it waits from now on. */
  opened(generation: number): void {
    this.generation = generation;
    this.moved = false;
    this.waiting = [];
    this.waitingBytes = 0;
  }

  /** A message from the session: what to take now, in order, or null when too much waits (give the session up). */
  fromPath(m: ServerMessage, size: number): ServerMessage[] | null {
    if (this.moved) return [m];
    this.waitingBytes += size;
    if (this.waitingBytes > MAX_BUFFERED) return null;
    this.waiting.push(m);
    return [];
  }

  /** `media-moved`: what waited, to take now, when it names the open session. */
  movedTo(generation: number): ServerMessage[] {
    if (generation !== this.generation || this.moved) return [];
    this.moved = true;
    const waiting = this.waiting;
    this.waiting = [];
    this.waitingBytes = 0;
    return waiting;
  }

  /** The session ended (it closed, or `media-fallback` came): what waited is dropped. */
  closed(): void {
    this.generation = null;
    this.moved = false;
    this.waiting = [];
    this.waitingBytes = 0;
  }

  /** `media-fallback`: the media are back on the WebSocket, from a new epoch on. */
  fellBack(): void {
    this.closed();
    this.awaitingConfig = true;
  }

  /** Whether the viewer takes media message `m` (anything else it always takes). */
  takes(m: ServerMessage): boolean {
    if (!this.awaitingConfig || !MEDIA.has(m.type)) return true;
    if (m.type !== 'config') return false;
    this.awaitingConfig = false;
    return true;
  }
}

export interface Session {
  /** Sends an acknowledgement or a keyframe request over the session. */
  send(m: ClientMessage): void;
  close(): void;
}

export interface SessionEvents {
  /** A message of the media stream, and its size on the wire. */
  media(m: ServerMessage, size: number): void;
  sound(m: MessageOf<'audio'>, size: number): void;
  /** The session ended, or broke the protocol. */
  closed(): void;
}

/** Opens offer `offer`'s session; null when it does not open within OPEN_TIMEOUT_MS (UDP blocked, say). */
export async function openSession(offer: MessageOf<'udp-offer'>, events: SessionEvents): Promise<Session | null> {
  let transport: WebTransport;
  try {
    transport = new WebTransport(offer.url, {
      serverCertificateHashes: [{ algorithm: 'sha-256', value: Uint8Array.from(offer.certSha256) }],
    });
  } catch {
    return null;
  }
  // A failed session rejects these: handled here, never left unhandled.
  transport.closed.catch(() => undefined);
  const ready = transport.ready.then(
    () => 'ready' as const,
    () => 'failed' as const,
  );
  const opened = await awaitAtMost<'ready' | 'failed' | 'timeout'>(ready, OPEN_TIMEOUT_MS, () => 'timeout');
  if (opened !== 'ready') {
    transport.close();
    return null;
  }
  let writer: WritableStreamDefaultWriter<Uint8Array>;
  try {
    writer = (await transport.createUnidirectionalStream()).getWriter();
  } catch {
    transport.close();
    return null;
  }
  let ended = false;
  const end = (): void => {
    if (ended) return;
    ended = true;
    transport.close();
    events.closed();
  };
  void transport.closed.then(
    () => end(),
    () => end(),
  );
  void readMedia(transport, events).then(end, end);
  void readSound(transport, events).catch(end);
  return {
    send(m: ClientMessage): void {
      if (!ended) writer.write(frame(m)).catch(() => end());
    },
    close: () => end(),
  };
}

/** The media stream: the first one the server opens. */
async function readMedia(transport: WebTransport, events: SessionEvents): Promise<void> {
  const streams = transport.incomingUnidirectionalStreams.getReader();
  const { value: stream, done } = await streams.read();
  streams.releaseLock();
  if (done || stream === undefined) return;
  const reader = stream.getReader();
  const frames = new Frames();
  for (;;) {
    const { value, done: finished } = await reader.read();
    if (finished) return;
    for (const body of frames.push(value)) events.media(decode(body), body.length + 5);
  }
}

/** Sound: one message per datagram; anything else in one breaks the protocol. */
async function readSound(transport: WebTransport, events: SessionEvents): Promise<void> {
  const reader = transport.datagrams.readable.getReader();
  for (;;) {
    const { value, done } = await reader.read();
    if (done) return;
    const m = decode(value as Uint8Array);
    if (m.type !== 'audio') throw new Error(`a ${m.type} message in a datagram`);
    events.sound(m, (value as Uint8Array).length);
  }
}
