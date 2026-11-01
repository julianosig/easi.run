// The film (src/video/Film.svelte): every feature's headline over its drawing, captured frame by frame and encoded by
// ffmpeg. Run in the benchmark runner's Playwright image, from the repository's root:
//
//   docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -e OUT="$OUT" -v "$PWD:/w" -v "$OUT:$OUT" -w /w \
//     mcr.microsoft.com/playwright:v1.61.1-noble \
//     bash -c 'npm ci -s --prefix bench/runner && node site/scripts/film.mjs --out=$OUT [--locale=en] [--at=1,8,20]'
//
// `--at` takes stills at those seconds (to look at the frames); otherwise every frame goes to <out>/frames/ as JPEG,
// for: ffmpeg -framerate 30 -i <out>/frames/%05d.jpg ... (see scripts/film.sh).
import { mkdir, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const site = join(dirname(fileURLToPath(import.meta.url)), '..');
const { chromium } = createRequire(join(site, '../bench/runner/package.json'))('playwright');
const { createServer } = await import(join(site, 'node_modules/vite/dist/node/index.js'));

const option = (name, otherwise) => process.argv.find((a) => a.startsWith(`--${name}=`))?.slice(name.length + 3) ?? otherwise;
const out = option('out', '/tmp/film');
const locale = option('locale', 'en');
const at = option('at', '');
const FPS = 30;

// The messages plugin reads its catalogs from the working directory.
process.chdir(site);
const server = await createServer({ root: site, configFile: join(site, 'vite.config.ts'), server: { port: 5199, host: '127.0.0.1', fs: { allow: [site, join(site, '..')] } }, logLevel: 'warn' });
await server.listen();
const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: { width: 1920, height: 1080 } });
  page.on('pageerror', (e) => console.error('page:', e.message));
  await page.goto(`http://127.0.0.1:5199/video/?capture&locale=${locale}`);
  await page.waitForFunction(() => 'film' in window);
  await page.evaluate(() => window.film.ready());
  const duration = await page.evaluate(() => window.film.duration);
  console.log(`film: ${duration.toFixed(1)} s`);
  if (at) {
    await mkdir(out, { recursive: true });
    for (const second of at.split(',').map(Number)) {
      await page.evaluate((s) => window.film.seek(s), second);
      await page.screenshot({ path: join(out, `at-${second}.png`) });
    }
  } else {
    const frames = join(out, 'frames');
    await rm(frames, { recursive: true, force: true });
    await mkdir(frames, { recursive: true });
    const total = Math.ceil(duration * FPS);
    for (let i = 0; i < total; i++) {
      await page.evaluate((s) => window.film.seek(s), i / FPS);
      await page.screenshot({ path: join(frames, `${String(i).padStart(5, '0')}.jpg`), type: 'jpeg', quality: 95 });
      if (i % 300 === 0) console.log(`frame ${i}/${total}`);
    }
  }
} finally {
  await browser.close();
  await server.close();
}
