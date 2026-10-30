// Makes display/src/logo.qoi, the logo the output shows where no surface is (display/src/background.rs): the mark and
// the name as the landing's Wordmark draws them (site/src/lib/Wordmark.svelte with its mark), in the landing's font,
// on a transparent ground, cropped to what it draws. Its width is the most the display ever draws it at, so a large
// output shows it pixel for pixel. Run it from the checkout's root, in Playwright's image (it installs its two
// packages, pinned, from npm):
//
//   docker run --rm --network host -u $(id -u) -e HOME=/tmp -v $PWD:/w -w /w \
//     mcr.microsoft.com/playwright:v1.62.0-noble@sha256:baed2032d533817f3dbe6425de795788430ba345e819a1201337009ba17c9d07 \
//     node display/logo.mjs
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

/** The logo's width in pixels. */
const WIDTH = 320;
const OUT = 'display/src/logo.qoi';

const dir = mkdtempSync(join(tmpdir(), 'logo-'));
execFileSync('npm', ['install', '--silent', '--no-save', '--prefix', dir, 'playwright-core@1.62.0', '@fontsource-variable/plus-jakarta-sans@5.3.0'], { stdio: 'inherit' });
const require = createRequire(join(dir, 'node_modules/'));
const { chromium } = require('playwright-core');
const font = readFileSync(join(dir, 'node_modules/@fontsource-variable/plus-jakarta-sans/files/plus-jakarta-sans-latin-wght-normal.woff2'));

// The Wordmark's markup and classes, spelled out: the tokens of client/src/app.css (ink, accent, you) and the mark of
// client/src/ui/Mark.svelte.
const page = (size) => `<!doctype html><style>
  @font-face { font-family: Jakarta; font-weight: 200 800; src: url(data:font/woff2;base64,${font.toString('base64')}) format('woff2'); }
  html, body { margin: 0; background: transparent; }
  #logo { display: inline-block; padding: 8px; line-height: 1; white-space: nowrap; font: 600 ${size}px Jakarta;
    letter-spacing: -0.03em; color: #14284b; }
  svg { display: inline-block; height: 1.11em; width: auto; margin-right: 0.45em; vertical-align: -0.28em; }
</style><span id="logo"><svg viewBox="10 4 48 56"><path d="M12 6 56 30 35 36 25 58Z" fill="#2457f5" stroke="#2457f5"
  stroke-width="3.5" stroke-linejoin="round"/><circle cx="51" cy="51" r="6.5" fill="#f06445"/></svg>easi<span
  style="color: #f06445">.</span>run</span>`;

const browser = await chromium.launch();
const tab = await browser.newPage({ viewport: { width: 1600, height: 600 }, deviceScaleFactor: 1 });
/** The logo drawn at `size` pixels a letter, cropped to its drawn pixels: `[width, height, RGBA]`. */
async function draw(size) {
  await tab.setContent(page(size));
  await tab.evaluate(() => document.fonts.ready);
  const png = await tab.locator('#logo').screenshot({ omitBackground: true });
  return tab.evaluate(async (bytes) => {
    const image = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }));
    const canvas = new OffscreenCanvas(image.width, image.height);
    const context = canvas.getContext('2d');
    context.drawImage(image, 0, 0);
    const { data } = context.getImageData(0, 0, image.width, image.height);
    const alpha = (x, y) => data[(y * image.width + x) * 4 + 3];
    let [x0, y0, x1, y1] = [image.width, image.height, -1, -1];
    for (let y = 0; y < image.height; y++) {
      for (let x = 0; x < image.width; x++) {
        if (alpha(x, y)) [x0, y0, x1, y1] = [Math.min(x0, x), Math.min(y0, y), Math.max(x1, x), Math.max(y1, y)];
      }
    }
    const [w, h] = [x1 - x0 + 1, y1 - y0 + 1];
    const rgba = [];
    for (let y = y0; y <= y1; y++) rgba.push(...data.subarray((y * image.width + x0) * 4, (y * image.width + x1 + 1) * 4));
    return [w, h, rgba];
  }, [...png]);
}
// The size whose drawing is WIDTH wide: measured at 100 pixels, then corrected once for rounding.
let size = 100;
let [w, h, rgba] = await draw(size);
for (let i = 0; i < 4 && w !== WIDTH; i++) {
  size *= WIDTH / w;
  [w, h, rgba] = await draw(size);
}
await browser.close();
if (w > WIDTH) throw new Error(`the logo is ${w} pixels wide, more than ${WIDTH}`);

/** `rgba` (w × h straight RGBA) as a QOI image (https://qoiformat.org/qoi-specification.pdf). */
function qoi(w, h, rgba) {
  const out = [...Buffer.from('qoif'), w >>> 24, (w >> 16) & 255, (w >> 8) & 255, w & 255, h >>> 24, (h >> 16) & 255,
    (h >> 8) & 255, h & 255, 4, 0];
  const index = new Array(64).fill(-1);
  let [pr, pg, pb, pa] = [0, 0, 0, 255];
  let run = 0;
  const signed = (d) => ((d + 128) & 255) - 128;
  for (let i = 0; i < rgba.length; i += 4) {
    const [r, g, b, a] = rgba.slice(i, i + 4);
    if (r === pr && g === pg && b === pb && a === pa) {
      run++;
      if (run === 62 || i + 4 === rgba.length) {
        out.push(0xc0 | (run - 1));
        run = 0;
      }
      continue;
    }
    if (run > 0) {
      out.push(0xc0 | (run - 1));
      run = 0;
    }
    const slot = (r * 3 + g * 5 + b * 7 + a * 11) % 64;
    const key = ((r << 24) | (g << 16) | (b << 8) | a) >>> 0;
    if (index[slot] === key) {
      out.push(slot);
    } else {
      index[slot] = key;
      const [vr, vg, vb] = [signed(r - pr), signed(g - pg), signed(b - pb)];
      const [vgr, vgb] = [vr - vg, vb - vg];
      if (a !== pa) {
        out.push(0xff, r, g, b, a);
      } else if (vr >= -2 && vr <= 1 && vg >= -2 && vg <= 1 && vb >= -2 && vb <= 1) {
        out.push(0x40 | ((vr + 2) << 4) | ((vg + 2) << 2) | (vb + 2));
      } else if (vg >= -32 && vg <= 31 && vgr >= -8 && vgr <= 7 && vgb >= -8 && vgb <= 7) {
        out.push(0x80 | (vg + 32), ((vgr + 8) << 4) | (vgb + 8));
      } else {
        out.push(0xfe, r, g, b);
      }
    }
    [pr, pg, pb, pa] = [r, g, b, a];
  }
  out.push(0, 0, 0, 0, 0, 0, 0, 1);
  return Buffer.from(out);
}

const file = qoi(w, h, rgba);
writeFileSync(OUT, file);
console.log(`${OUT}: ${w}x${h}, ${file.length} bytes`);
