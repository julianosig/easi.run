// Captures one language's film frame by frame: <out>/<locale>/film.html (make.py stage), seeked with render(t), each frame
// a JPEG in <out>/<locale>/frames/. Run by make.py in the Playwright image, from the repository's root:
//   node site/film/render.mjs <out> <locale> <workers> <fps>
import { mkdir, readFile, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { join } from 'node:path';

const [out, locale, workers, fps] = process.argv.slice(2);
const { chromium } = createRequire(join(process.cwd(), 'bench/runner/package.json'))('playwright');
const dir = join(out, locale);
const timing = JSON.parse(await readFile(join(dir, 'timing.json'), 'utf8'));
const total = Math.ceil(timing.end * Number(fps));
await rm(join(dir, 'frames'), { recursive: true, force: true });
await mkdir(join(dir, 'frames'));
const browser = await chromium.launch();
let next = 0;
await Promise.all(
  Array.from({ length: Number(workers) }, async () => {
    const page = await browser.newPage({ viewport: { width: 1920, height: 1080 } });
    page.on('pageerror', (error) => {
      throw error;
    });
    await page.goto(`file://${join(dir, 'film.html')}`);
    await page.evaluate(() => window.ready);
    for (let i = next++; i < total; i = next++) {
      await page.evaluate((t) => window.render(t), i / Number(fps));
      await page.screenshot({ path: join(dir, 'frames', `${String(i).padStart(5, '0')}.jpg`), type: 'jpeg', quality: 92 });
    }
    await page.close();
  }),
);
await browser.close();
console.log(`${locale}: ${total} frames`);
