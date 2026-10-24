// Measures one system on one fixture from a headless Chromium client and prints one JSON object.
//   LOGIN_COOKIE=... LOGIN_CSRF=... node measure.mjs easi-run ORIGIN SESSION FIXTURE SECONDS WARMUP OUT
//   node measure.mjs baseline  KASMVNC_BASE_URL - FIXTURE SECONDS WARMUP OUT
// Both clients are measured the same way: every drawn frame's time strip is decoded from the canvas the person sees
// (capture-to-draw), distinct stamps count presented frames, WebSocket payload bytes come from the DevTools protocol,
// and the typing fixture times a key press to its square flipping in a drawn frame.
import { chromium } from 'playwright';
import { writeFileSync } from 'node:fs';

const [system, target, session, fixture, seconds, warmup, out] = process.argv.slice(2);
// HEADED=1 runs a headed client (under Xvfb in the runner's container); otherwise the client is headless.
const headed = process.env.HEADED === '1';
// This host has no GPU: a headed client runs without one too (`--disable-gpu`), rather than through SwiftShader, on
// which KasmVNC's client leaves large regions unpainted in what the person sees.
const browser = await chromium.launch({
  headless: !headed,
  args: ['--ignore-certificate-errors', '--disable-gpu-vsync', ...(headed ? ['--disable-gpu'] : [])],
});
const context = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1400, height: 900 } });
const page = await context.newPage();
// Every received WebSocket payload byte, and for easi.run the media messages alone (video, tiles, cursor: the
// first byte is the message kind), so the static-page gate excludes lease, stats, and clock messages.
const MEDIA = new Set([0x03, 0x04, 0x05]);
let wsBytes = 0;
let mediaBytes = 0;
page.on('websocket', (ws) => ws.on('framereceived', (f) => {
  const n = typeof f.payload === 'string' ? f.payload.length : f.payload.byteLength;
  wsBytes += n;
  if (typeof f.payload !== 'string' && (system !== 'easi-run' || MEDIA.has(f.payload[0]))) mediaBytes += n;
}));
const capture = ['quality', 'text-scroll'].includes(fixture) ? 1500 : 0;

// The baseline client's screen canvas gets the same measurement easi.run's bench page has built in.
const baselineProbe = `(() => {
  const SPAN = 2 ** 40, W = 704, H = 16;
  const b = { token: '', inputLatencies: [], presented: 0, stamped: 0, latencies: [], latencyAt: [], frames: [], stats: [], errors: [], start: Date.now() };
  window.__bench = b;
  let last = -1, flag = null, pressedAt = null, lastCapture = 0;
  window.addEventListener('keydown', () => (pressedAt ??= performance.now()), true);
  const bit = (d, n) => { const i = (8 * W + n * 16 + 8) * 4; const v = (d[i] + d[i + 1] + d[i + 2]) / 3; return v > 192 ? 1 : v < 64 ? 0 : null; };
  const loop = () => {
    try { probe(); } catch (e) { if (b.errors.length < 5) b.errors.push(String(e)); }
    requestAnimationFrame(loop);
  };
  const probe = () => {
    // The screen canvas (the client also has a small multi-monitor widget canvas).
    const canvas = [...document.querySelectorAll('canvas')].find((c) => c.width >= 1280);
    if (canvas) {
      const ctx = canvas.getContext('2d');
      const d = ctx.getImageData(0, 0, W, H).data;
      const f = ctx.getImageData(1140, 140, 1, 1).data[0] > 128;
      if (flag !== null && f !== flag && pressedAt !== null) { b.inputLatencies.push(performance.now() - pressedAt); pressedAt = null; }
      flag = f;
      if ([bit(d, 0), bit(d, 1), bit(d, 42), bit(d, 43)].join() === '1,0,0,1') {
        let v = 0, ok = true;
        for (let n = 2; n < 42; n++) { const x = bit(d, n); if (x === null) { ok = false; break; } v = v * 2 + x; }
        const now = Date.now(); const stamp = ok ? now - (((now % SPAN) - v + SPAN) % SPAN) : null;
        if (ok && v !== last) {
          last = v; b.presented++; b.stamped++;
          b.latencies.push(now - stamp);
          b.latencyAt.push(now);
        }
        // Quality frames on a fixed schedule, not at the moment a stamp changes: a client that draws a frame's
        // rectangles over several animation frames is captured as a person sees it at an ordinary moment.
        if (ok && ${capture} > 0 && performance.now() - lastCapture >= ${capture} && b.frames.length < 30) {
          lastCapture = performance.now();
          const c = document.createElement('canvas'); c.width = 1280; c.height = 800;
          c.getContext('2d').drawImage(canvas, 0, 0);
          b.frames.push({ stamp, png: c.toDataURL('image/png') });
        }
      }
    }
  };
  requestAnimationFrame(loop);
})();`;

let csrf = '';
if (system === 'easi-run') {
  // The runner's own login, shared by every run: one login per run would queue on the admin's password check.
  await context.addCookies([
    { name: '__Host-display', value: process.env.LOGIN_COOKIE, url: target, secure: true, httpOnly: true, sameSite: 'Strict' },
  ]);
  csrf = process.env.LOGIN_CSRF;
  await page.goto(`${target}/bench.html?session=${session}&capture_ms=${capture}`);
} else {
  // KasmVNC's own web client (its noVNC fork), as a KasmVNC user runs it. The target is the server's base URL. The
  // client's own defaults (dynamic quality 3-9, its minimum frame rate) override the server's, so the decision's
  // settings are passed to it as well.
  await page.addInitScript(baselineProbe);
  await page.goto(
    `${target}/vnc.html?autoconnect=1&resize=off&reconnect=0&framerate=60&dynamic_quality_min=7&dynamic_quality_max=9`,
  );
}
const snapshot = () => page.evaluate(() => {
  const b = window.__bench;
  return { presented: b.presented, stamped: b.stamped, latencies: b.latencies.length, inputs: b.inputLatencies.length, frames: b.frames.length };
});
await page.waitForTimeout(Number(warmup) * 1000);

if (fixture === 'typing') {
  if (system === 'easi-run') {
    const token = await page.evaluate(() => window.__bench.token);
    const granted = await context.request.post(`${target}/api/sessions/${session}/control`, {
      headers: { origin: target, 'x-csrf-token': csrf },
      data: { viewer: token },
    });
    if (!granted.ok()) throw new Error(`control: ${granted.status()} ${await granted.text()}`);
    await page.waitForTimeout(1000);
  }
  // Focus the remote text field the way a person does: a click on the screen.
  const box = await page.evaluate((selector) => {
    const canvas = [...document.querySelectorAll(selector)].find((c) => c.width >= 1280);
    const r = canvas.getBoundingClientRect();
    return { x: r.x, y: r.y, width: r.width, height: r.height };
  }, system === 'easi-run' ? '.screen canvas' : 'canvas');
  await page.mouse.click(box.x + 400 * (box.width / 1280), box.y + 62 * (box.height / 800));
  await page.waitForTimeout(500);
}

const start = await snapshot();
// The runner samples the containers' CPU counters on these two lines.
console.log('MARK start');
const bytesAtStart = wsBytes;
const mediaAtStart = mediaBytes;
const startedAt = Date.now();
if (fixture === 'typing') {
  const end = Date.now() + Number(seconds) * 1000;
  let n = 0;
  while (Date.now() < end) {
    await page.keyboard.press(n++ % 2 ? 'a' : 'b');
    await page.waitForTimeout(250);
  }
} else {
  await page.waitForTimeout(Number(seconds) * 1000);
}
const elapsed = (Date.now() - startedAt) / 1000;
function stages(stats) {
  const median = (values) => {
    const v = values.filter((x) => typeof x === 'number').sort((a, b) => a - b);
    return v.length ? v[Math.floor(v.length / 2)] / 1000 : null;
  };
  return {
    composeToArrive: median(stats.map((s) => s.composeToArriveUsP50)),
    composeToDraw: median(stats.map((s) => s.composeToDrawUsP50)),
    decode: median(stats.map((s) => s.decodeUsP50)),
    encode: median(stats.map((s) => s.server?.encodeUsP50)),
  };
}
console.log('MARK end');
const end = await snapshot();
const detail = await page.evaluate(([from, inputsFrom, since]) => {
  const b = window.__bench;
  return { latencyAt: (b.latencyAt ?? []).slice(from), latencies: b.latencies.slice(from), inputLatencies: b.inputLatencies.slice(inputsFrom), frames: b.frames, errors: b.errors, stats: b.stats.filter((s) => s.t >= since) };
}, [start.latencies, start.inputs, startedAt]);
detail.frames.forEach((f, i) => writeFileSync(`${out}/frame-${i}-${f.stamp}.png`, Buffer.from(f.png.split(',')[1], 'base64')));
const result = {
  system, fixture, seconds: elapsed,
  presentedFps: (end.stamped - start.stamped) / elapsed,
  drawsPerSecond: (end.presented - start.presented) / elapsed,
  kbitPerSecond: ((wsBytes - bytesAtStart) * 8) / 1000 / elapsed,
  bytes: wsBytes - bytesAtStart,
  mediaBytesPerSecond: (mediaBytes - mediaAtStart) / elapsed,
  headed,
  clientBrowser: browser.version(),
  // easi.run's own clock calibration (the best round trip's half), in microseconds; both ends share the host clock.
  clockErrorUs: detail.stats.map((s) => s.clockErrorUs).filter((v) => typeof v === 'number').at(-1) ?? null,
  latencies: detail.latencies,
  latencyAt: detail.latencyAt,
  inputLatencies: detail.inputLatencies,
  frames: detail.frames.map((f) => f.stamp),
  errors: detail.errors,
  // easi.run's own latency stages over the window (medians of its per-second statistics), in milliseconds.
  stages: system === 'easi-run' ? stages(detail.stats) : null,
};
writeFileSync(`${out}/result.json`, JSON.stringify(result));
console.log(JSON.stringify({ ...result, latencies: result.latencies.length, inputLatencies: result.inputLatencies.length }));
await browser.close();
