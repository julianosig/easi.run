// The benchmark client: the viewer, plus measurement of what it actually draws. Opened by bench/runner with
// ?session=<id>&capture_ms=<ms>; results accumulate on window.__bench.

import { LeaseHolder } from './protocol.ts';
import { mountViewer, viewerToken } from './viewer.ts';
import { decodeStamp, STAMP_HEIGHT, STAMP_WIDTH, unwrapStamp } from './stamp.ts';

interface Bench {
  token: string;
  /** Input-to-draw: from a key press to the typing fixture's square flipping in a drawn frame. */
  inputLatencies: number[];
  presented: number;
  stamped: number;
  latencies: number[];
  /** When each capture-to-draw sample was drawn (`Date.now()`), so a run's latency can be followed over time. */
  latencyAt: number[];
  frames: { stamp: number; png: string }[];
  stats: unknown[];
  errors: string[];
}

const params = new URLSearchParams(location.search);
const session = params.get('session') ?? '';
const captureEvery = Number(params.get('capture_ms') ?? '0');
const token = viewerToken();
const bench: Bench = {
  token,
  inputLatencies: [],
  presented: 0,
  stamped: 0,
  latencies: [],
  latencyAt: [],
  frames: [],
  stats: [],
  errors: [],
};
(window as unknown as { __bench: Bench }).__bench = bench;
// The typing fixture's square (right 40 px, top 40 px, 200x200 in a 1280-wide output): sample its middle.
const FLAG = { x: 1140, y: 140 };
let flag: boolean | null = null;
let pressedAt: number | null = null;
window.addEventListener('keydown', () => (pressedAt ??= performance.now()), true);
const screen = document.getElementById('screen') as HTMLElement;
let lastStamp = -1;
let lastCapture = 0;
const scratch = document.createElement('canvas');
scratch.width = 1280;
scratch.height = 800;
const scratchContext = scratch.getContext('2d')!;

const viewer = mountViewer(screen, {
  url: `wss://${location.host}/ws/sessions/${session}?viewer=${token}`,
  session,
  onStats: (s) => bench.stats.push({ t: Date.now(), ...s, server: s.server ? { ...s.server, serverTimeUs: String(s.server.serverTimeUs) } : null }),
  onError: (e) => bench.errors.push(e.message),
  // The runner grants control through the admin API; the viewer sends input while it holds the lease.
  onLease: (lease) => viewer.setControl(lease.holder === LeaseHolder.You),
  onPresent: (read) => {
    bench.presented++;
    const [r] = read(FLAG.x, FLAG.y, 1, 1);
    const white = r! > 128;
    if (flag !== null && white !== flag && pressedAt !== null) {
      bench.inputLatencies.push(performance.now() - pressedAt);
      pressedAt = null;
    }
    flag = white;
    const strip = read(0, 0, STAMP_WIDTH, STAMP_HEIGHT);
    const low = decodeStamp(strip);
    if (low === null) return;
    const now = Date.now();
    const stamp = unwrapStamp(low, now);
    if (low !== lastStamp) {
      lastStamp = low;
      bench.stamped++;
      bench.latencies.push(now - stamp);
      bench.latencyAt.push(now);
    }
    // Quality frames on a fixed schedule, as bench/runner/measure.mjs captures the baseline's.
    const elapsed = performance.now();
    if (captureEvery > 0 && elapsed - lastCapture >= captureEvery && bench.frames.length < 30) {
      lastCapture = elapsed;
      const pixels = read(0, 0, 1280, 800);
      const image = scratchContext.createImageData(1280, 800);
      image.data.set(pixels);
      scratchContext.putImageData(image, 0, 0);
      bench.frames.push({ stamp, png: scratch.toDataURL('image/png') });
    }
  },
});
