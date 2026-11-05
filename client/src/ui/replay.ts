// A session's replay (admin/src/replay.rs): its index, its segments' records, and the clock that plays them. Pure but
// for the time and the loading it is given, so the tests drive it: `Playback` feeds a sink (the screen) the records up
// to where it is, at a speed, skipping idle time when asked, and a seek starts the screen over from the segment that
// holds the moment.

/** What `GET /api/sessions/{id}/replay` answers. Times are milliseconds from `started` (Unix milliseconds). */
export interface ReplayIndex {
  started: number;
  /** `starts`: `watch` (the first of a connection to the display: a whole picture first), `refresh` (a cut that asked
   * for a whole picture as it began) or `more` (a cut for size, going on from the one before). */
  segments: { n: number; start: number; end: number; size: number; starts?: 'watch' | 'refresh' | 'more' }[];
  /** Why it stopped recording before its session ended (`unwritten`), if it did. */
  stopped: string | null;
  /** Whether it records now. */
  recording: boolean;
}

/** A record's kind: a display message (as a viewer got it), where the agent's pointer went, the start of a gap. */
export const Kind = { Message: 0, Pointer: 1, Gap: 2 } as const;
export interface ReplayRecord {
  t: number;
  kind: number;
  bytes: Uint8Array;
}

/** A segment's records: when (u32), kind (u8), length (u32), bytes, each big-endian; a record cut short ends them. */
export function parseRecords(data: Uint8Array): ReplayRecord[] {
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  const records: ReplayRecord[] = [];
  let at = 0;
  while (at + 9 <= data.length) {
    const t = view.getUint32(at);
    const kind = view.getUint8(at + 4);
    const length = view.getUint32(at + 5);
    if (at + 9 + length > data.length) break;
    records.push({ t, kind, bytes: data.subarray(at + 9, at + 9 + length) });
    at += 9 + length;
  }
  return records;
}

/** Where the agent's pointer is in a pointer record, or none (the agent stopped pointing). */
export function pointerOf(bytes: Uint8Array): { x: number; y: number } | null {
  if (bytes.length !== 4) return null;
  const x = (bytes[0]! << 8) | bytes[1]!;
  const y = (bytes[2]! << 8) | bytes[3]!;
  return x === 0xffff && y === 0xffff ? null : { x, y };
}

/** How long the replay lasts: to the last segment's end. */
export const duration = (index: ReplayIndex): number => index.segments.at(-1)?.end ?? 0;

/** The segment that holds moment `t`: the last that starts at or before it (the first, before any). */
export function segmentFor(index: ReplayIndex, t: number): number {
  let found = 0;
  index.segments.forEach((s, i) => {
    if (s.start <= t) found = i;
  });
  return found;
}

/** The segment a seek to `t` starts feeding from: back over the segments that follow on one from another, to the
 * first of its watch (a whole picture first), or to the second refresh before the moment: the refresh the nearer one
 * asked for may still be arriving, the one before has had a whole segment to arrive. */
export function seekFrom(index: ReplayIndex, t: number): number {
  let at = segmentFor(index, t);
  let refreshes = 0;
  for (;;) {
    const s = index.segments[at];
    if (s === undefined || (s.starts ?? 'watch') === 'watch') return at;
    if (s.starts === 'refresh' && ++refreshes === 2) return at;
    const before = index.segments[at - 1];
    if (before === undefined || s.start - before.end > 50) return at;
    at -= 1;
  }
}

/** The stretches with nothing recorded (a person had control, or the recorder waited for the agent), as [start, end]. */
export function unrecorded(index: ReplayIndex): [number, number][] {
  const gaps: [number, number][] = [];
  // Its start, when it was deleted for room.
  const first = index.segments[0]?.start ?? 0;
  if (first > 1000) gaps.push([0, first]);
  for (let i = 1; i < index.segments.length; i++) {
    const before = index.segments[i - 1]!.end;
    const next = index.segments[i]!.start;
    if (next - before > 1000) gaps.push([before, next]);
  }
  return gaps;
}

/** `m:ss` (or `h:mm:ss`) for a replay time. */
export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const mm = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(mm).padStart(2, '0')}:${ss}` : `${mm}:${ss}`;
}

/** Where the replay's records go: the screen. */
export interface Sink {
  reset(): void;
  message(bytes: Uint8Array): void;
  pointer(x: number, y: number): void;
}

/** Past this much time with nothing on screen, skipping idle time jumps to just before the next record. */
export const IDLE_MS = 1500;
/** Where an idle jump lands, before the next record. */
const LEAD_MS = 300;

/** Plays a replay into a sink: `advance` moves its time on by the real milliseconds elapsed, at its speed. A segment
 * that does not load stops it (`onError`); a seek tries again. */
/** A display message's first byte, when it draws (see `protocol`). */
const VIDEO = 0x03;
const TILES = 0x04;

export class Playback {
  time = 0;
  playing = false;
  speed = 1;
  skipIdle = true;
  /** The segment fed from (an index into the index's segments), and the next of its records to feed. */
  private segment = -1;
  private records: ReplayRecord[] | null = null;
  private next = 0;
  /** The first picture of the segment a seek opened: it is fed however early the moment is, as before it the screen would
   * have nothing to show. */
  private picture = -1;
  /** Set while a seek or a segment loads: `advance` waits for it. */
  private loading: Promise<void> | null = null;
  /** Each seek's own number: a load an older one started feeds nothing. */
  private turn = 0;

  /** What it plays, which a running session's recording lengthens and the disk's limit shortens (replaced as a whole). */
  private current: ReplayIndex;

  constructor(
    index: ReplayIndex,
    private readonly load: (n: number) => Promise<ReplayRecord[]>,
    private readonly sink: Sink,
    private readonly onError: (error: unknown) => void = () => {},
  ) {
    this.current = index;
  }

  /** Stops it for good: what is still loading feeds nothing. */
  dispose(): void {
    this.turn++;
    this.playing = false;
    this.loading = null;
  }

  get end(): number {
    return duration(this.index);
  }

  /** The recording's index, read again: the segment it feeds from keeps its place, and if that was deleted for room, it
   * goes on from what is left. */
  get index(): ReplayIndex {
    return this.current;
  }
  set index(found: ReplayIndex) {
    const feeding = this.current.segments[this.segment]?.n;
    this.current = found;
    if (feeding === undefined) return;
    this.segment = found.segments.findIndex((s) => s.n === feeding);
    if (this.segment < 0) void this.seek(this.time);
  }

  /** Starts the screen over from the segment that holds `t` and feeds it up to `t`. */
  seek(t: number): Promise<void> {
    const mine = ++this.turn;
    // What was deleted for room is gone: the earliest moment left is where it starts.
    this.time = Math.max(this.index.segments[0]?.start ?? 0, Math.min(t, this.end));
    const segment = seekFrom(this.index, this.time);
    const meta = this.index.segments[segment];
    if (meta === undefined) return Promise.resolve();
    this.loading = this.load(meta.n).then(
      (records) => {
        if (mine !== this.turn) return;
        this.sink.reset();
        // By its number: the index may have lost its head while this loaded.
        this.segment = this.current.segments.findIndex((s) => s.n === meta.n);
        if (this.segment < 0) return this.seek(this.time);
        this.records = records;
        this.picture = records.findIndex((r) => r.kind === Kind.Message && (r.bytes[0] === VIDEO || r.bytes[0] === TILES));
        this.next = 0;
        this.loading = null;
        // Settled once it reached the moment, the segments it crossed loaded.
        return this.feed();
      },
      (error: unknown) => this.failed(mine, error),
    );
    return this.loading;
  }

  /** Moves on by `elapsed` real milliseconds while playing, feeding what is due; stops at the end. */
  advance(elapsed: number): void {
    if (!this.playing || this.loading !== null || this.records === null) return;
    this.time = Math.min(this.end, this.time + elapsed * this.speed);
    const upcoming = this.upcoming();
    if (this.skipIdle && upcoming !== null && upcoming - this.time > IDLE_MS) this.time = upcoming - LEAD_MS;
    this.feed();
    if (this.time >= this.end) this.playing = false;
  }

  /** A load of seek `mine` failed: it stops where it is, and says why. */
  private failed(mine: number, error: unknown): void {
    if (mine !== this.turn) return;
    this.loading = null;
    this.playing = false;
    this.onError(error);
  }

  /** When the next record is due: in this segment, or the next segment's start. */
  private upcoming(): number | null {
    const record = this.records?.[this.next];
    if (record !== undefined) return record.t;
    return this.index.segments[this.segment + 1]?.start ?? null;
  }

  /** Feeds every record due by `time`, on into the next segments (loaded as they are reached): settled once it has. */
  private feed(): Promise<void> | void {
    for (;;) {
      const records = this.records;
      if (records === null) return;
      while (this.next < records.length && (records[this.next]!.t <= this.time || this.next <= this.picture)) {
        const r = records[this.next++]!;
        if (r.kind === Kind.Message) this.sink.message(r.bytes);
        else if (r.kind === Kind.Pointer) {
          const at = pointerOf(r.bytes);
          if (at) this.sink.pointer(at.x, at.y);
        } else if (r.kind === Kind.Gap) {
          // The recorder's watch ended there: the next one is a new connection, its stream numbered afresh.
          this.sink.reset();
        }
      }
      if (this.next < records.length) return;
      const following = this.index.segments[this.segment + 1];
      if (following === undefined || following.start > this.time) return;
      // On into the next segment, which goes on from this one's picture (its configuration is the same).
      const mine = this.turn;
      this.records = null;
      this.loading = this.load(following.n).then(
        (loaded) => {
          if (mine !== this.turn) return;
          this.segment += 1;
          this.records = loaded;
          this.next = 0;
          this.picture = -1;
          this.loading = null;
          return this.feed();
        },
        (error: unknown) => this.failed(mine, error),
      );
      return this.loading;
    }
  }
}
