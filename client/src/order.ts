// Ordering and recovery of one viewer stream (docs/design.md §3.3), with no I/O.
//
// Messages arrive in stream order, each video and tiles message with a greater revision than the one before it (a full
// refresh larger than one message is a run of tile messages of successive revisions). Tiles decode synchronously and
// are drawn on arrival; video frames decode asynchronously, so a frame can come out of the decoder after newer lossless
// tiles were drawn. Such a late frame is drawn only where no tile of a higher revision was drawn since it was queued.

import { Codec, type MessageOf, type Rect } from './protocol.ts';

/** A message that breaks the stream's contract; the viewer closes the stream. */
export class OrderError extends Error {
  override name = 'OrderError';
}

/** The least time between two keyframe requests while the stream stays undecodable. */
export const KEYFRAME_RETRY_MS = 1000;
/** Tile rectangles remembered for late video frames before those frames are skipped instead. */
export const MAX_REMEMBERED_TILES = 16384;

type Verdict = 'apply' | 'stale';

type VideoVerdict =
  | { kind: 'decode'; seq: number; keyframe: boolean }
  | { kind: 'drop'; keyframeRequest: number | null };

type Screen = Omit<MessageOf<'config'>, 'type' | 'version'>;

interface Pending {
  revision: bigint;
  rect: Rect;
}

function contains(outer: { width: number; height: number }, r: Rect): boolean {
  return r.x + r.w <= outer.width && r.y + r.h <= outer.height;
}

function intersects(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

/** `a` minus `b` as at most four disjoint rectangles (bands above and below, then left and right). */
export function subtract(a: Rect, b: Rect): Rect[] {
  if (!intersects(a, b)) return [a];
  const out: Rect[] = [];
  const top = Math.max(a.y, b.y);
  const bottom = Math.min(a.y + a.h, b.y + b.h);
  if (b.y > a.y) out.push({ x: a.x, y: a.y, w: a.w, h: b.y - a.y });
  if (a.y + a.h > bottom) out.push({ x: a.x, y: bottom, w: a.w, h: a.y + a.h - bottom });
  if (b.x > a.x) out.push({ x: a.x, y: top, w: b.x - a.x, h: bottom - top });
  if (a.x + a.w > b.x + b.w) out.push({ x: b.x + b.w, y: top, w: a.x + a.w - (b.x + b.w), h: bottom - top });
  return out;
}

export function subtractAll(rect: Rect, holes: Iterable<Rect>): Rect[] {
  let parts = [rect];
  for (const hole of holes) {
    parts = parts.flatMap((part) => subtract(part, hole));
    if (parts.length === 0) break;
  }
  return parts;
}

export class StreamOrder {
  private current: Screen | null = null;
  private highest = 0n;
  private placement: Rect | null = null;
  private awaitingKeyframe = true;
  private lastRequest = -Infinity;
  private nextSeq = 0;
  private readonly pending = new Map<number, Pending>();
  private drawn: { rect: Rect; revision: bigint }[] = [];

  get screen(): Screen | null {
    return this.current;
  }

  /**
   * A configuration. A new epoch resets everything (the first configuration of a connection included, which covers
   * reconnects) and needs no keyframe request: the display opens every epoch with a full refresh and its video with
   * a keyframe, so a request is due only once the epoch's first second has passed undecodable. An older epoch is
   * stale; repeating the current one must repeat its configuration exactly.
   */
  config(m: MessageOf<'config'>, now: number): { verdict: 'reset' | 'same' | 'stale' } {
    const screen: Screen = { epoch: m.epoch, width: m.width, height: m.height, codec: m.codec, maxFps: m.maxFps };
    if (this.current !== null) {
      if (m.epoch < this.current.epoch) return { verdict: 'stale' };
      if (m.epoch === this.current.epoch) {
        const c = this.current;
        if (c.width !== m.width || c.height !== m.height || c.codec !== m.codec || c.maxFps !== m.maxFps) {
          throw new OrderError('the configuration changed without a new epoch');
        }
        return { verdict: 'same' };
      }
    }
    this.current = screen;
    this.highest = 0n;
    this.placement = null;
    this.awaitingKeyframe = true;
    this.pending.clear();
    this.drawn = [];
    this.lastRequest = now;
    return { verdict: 'reset' };
  }

  /** Validates a message's epoch: older is stale, newer than the configuration breaks the stream. */
  private screenFor(epoch: number): Screen | null {
    if (this.current === null) throw new OrderError('media before any configuration');
    if (epoch < this.current.epoch) return null;
    if (epoch > this.current.epoch) throw new OrderError('media of an epoch that was never configured');
    return this.current;
  }

  private inOrder(revision: bigint): boolean {
    if (revision < this.highest) return false;
    this.highest = revision;
    return true;
  }

  /** Asks for a keyframe while undecodable, at most once per `KEYFRAME_RETRY_MS`. */
  keyframeRequest(now: number): number | null {
    if (!this.awaitingKeyframe || this.current === null || now - this.lastRequest < KEYFRAME_RETRY_MS) return null;
    this.lastRequest = now;
    return this.current.epoch;
  }

  /** A video message: hand it to the decoder under `seq`, or drop it. */
  video(m: MessageOf<'video'>, now: number): VideoVerdict {
    const screen = this.screenFor(m.epoch);
    if (screen === null) return { kind: 'drop', keyframeRequest: null };
    if (screen.codec === Codec.None) throw new OrderError('video in a configuration without a codec');
    if (!contains(screen, m.rect)) throw new OrderError('video placement outside the screen');
    if (!this.inOrder(m.revision)) return { kind: 'drop', keyframeRequest: null };
    if (m.keyframe) {
      this.placement = m.rect;
      this.awaitingKeyframe = false;
    } else if (this.awaitingKeyframe) {
      return { kind: 'drop', keyframeRequest: this.keyframeRequest(now) };
    } else if (!sameRect(this.placement, m.rect)) {
      // An inter frame cannot move its region: the decoder state belongs to the keyframe's placement.
      return this.lose(now);
    }
    const seq = this.nextSeq++;
    this.pending.set(seq, { revision: m.revision, rect: m.rect });
    return { kind: 'decode', seq, keyframe: m.keyframe };
  }

  /**
   * A decoded frame came out of the decoder. Returns its placement and the parts of it to draw (empty when newer
   * tiles cover it wholly), or null when the frame is stale (another epoch, a decoder reset).
   */
  videoOutput(seq: number): { rect: Rect; parts: Rect[] } | null {
    const frame = this.pending.get(seq);
    if (frame === undefined) return null;
    this.pending.delete(seq);
    const newer = this.drawn.filter((t) => t.revision > frame.revision).map((t) => t.rect);
    const parts = subtractAll(frame.rect, newer);
    let oldest: bigint | null = null;
    for (const p of this.pending.values()) if (oldest === null || p.revision < oldest) oldest = p.revision;
    this.drawn = oldest === null ? [] : this.drawn.filter((t) => t.revision > oldest);
    return { rect: frame.rect, parts };
  }

  /** The decoder failed or was overrun: forget its queue and ask for a keyframe. */
  decoderLost(now: number): number | null {
    return this.lose(now).keyframeRequest;
  }

  private lose(now: number): { kind: 'drop'; keyframeRequest: number | null } {
    this.awaitingKeyframe = true;
    this.pending.clear();
    this.drawn = [];
    this.lastRequest = -Infinity;
    return { kind: 'drop', keyframeRequest: this.keyframeRequest(now) };
  }

  /** A tiles message: drawn at once when current; remembered against frames still in the decoder. */
  tiles(m: MessageOf<'tiles'>): Verdict {
    const screen = this.screenFor(m.epoch);
    if (screen === null) return 'stale';
    let area = 0;
    for (const rect of m.rects) {
      if (!contains(screen, rect)) throw new OrderError('tile outside the screen');
      area += rect.w * rect.h;
    }
    // Merged tiles may overlap a little; more than twice the screen in one message is not a stream the server sends.
    if (area > 2 * screen.width * screen.height) throw new OrderError('tiles cover more than twice the screen');
    if (!this.inOrder(m.revision)) return 'stale';
    if (this.pending.size > 0) {
      for (const rect of m.rects) this.drawn.push({ rect, revision: m.revision });
      if (this.drawn.length > MAX_REMEMBERED_TILES) {
        // Skip the queued frames rather than remember more: they are older than these tiles, and the decoder keeps
        // its state, so the next frame is drawn normally.
        this.pending.clear();
        this.drawn = [];
      }
    }
    return 'apply';
  }

  /** A cursor message of the current epoch applies; an older one is stale. */
  cursor(m: MessageOf<'cursor'>): Verdict {
    return this.screenFor(m.epoch) === null ? 'stale' : 'apply';
  }
}

function sameRect(a: Rect | null, b: Rect): boolean {
  return a !== null && a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
}
