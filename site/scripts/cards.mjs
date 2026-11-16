// The link previews: one 1200×630 card per page and language, public/og/<locale>/<page>.<hash>.png, drawn from the
// page's own words (its headline, as the messages say it) in the site's colors and type; the 180×180 home-screen icon,
// public/apple-touch-icon.png; and public/favicon.ico. Each card's name carries a hash of its pixels, since the
// platforms keep a preview image by its address for good; src/lib/cards.json lists the names, for the pages' og:image
// (src/App.svelte), and scripts/prerender.mjs refuses a build that lacks one. The headline stands in the middle: X
// lays the page's title over the card's top, and small previews crop to the middle square.
//
// Run it after changing those words, in the benchmark runner's Playwright image (its `playwright` package is that
// image's version), from the repository's root:
//
//   docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -v "$PWD:/w" -w /w \
//     mcr.microsoft.com/playwright:v1.61.1-noble \
//     bash -c 'npm ci -s --prefix bench/runner && node site/scripts/cards.mjs'
//
// The type comes from Google Fonts while it draws: Plus Jakarta Sans, as the site, and Noto Sans for the scripts it
// lacks.
import { createHash } from 'node:crypto';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { LOCALES } from '../../i18n/locales.ts';

const site = join(dirname(fileURLToPath(import.meta.url)), '..');
const { chromium } = createRequire(join(site, '../bench/runner/package.json'))('playwright');

const words = async (locale, file) => JSON.parse(await readFile(join(site, 'messages', locale, `${file}.json`), 'utf8'));

/** Each page's card: a small label over its headline, the headline's second part in blue. */
async function card(locale, page) {
  const [home, features, compare] = await Promise.all(['page', 'features', 'compare'].map((f) => words(locale, f)));
  if (page === 'home') return { label: '', lines: [home['hero.title'], home['hero.accent']] };
  // The label is the page's heading, without the full stop it ends with there.
  const label = (heading) => heading.replace(/[.。।]$/u, '');
  if (page === 'features') return { label: label(features['title']), lines: [features['card']] };
  return { label: label(compare['title']), lines: [compare['meta.title']] };
}

const escape = (s) => s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
const MARK = '<svg viewBox="10 4 48 56"><path d="M12 6 56 30 35 36 25 58Z" fill="#2457f5" stroke="#2457f5" stroke-width="3.5" stroke-linejoin="round"/><circle cx="51" cy="51" r="6.5" fill="#f06445"/></svg>';
const FONTS =
  'https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@500;700&family=Noto+Sans+SC:wght@500;700&family=Noto+Sans+JP:wght@500;700&family=Noto+Sans+KR:wght@500;700&family=Noto+Sans+Devanagari:wght@500;700&display=block';
/** The fallbacks each language's script needs, after the site's own type. */
const SCRIPT = { 'zh-CN': 'Noto Sans SC', ja: 'Noto Sans JP', ko: 'Noto Sans KR', hi: 'Noto Sans Devanagari' };

function html(locale, { label, lines }) {
  const family = ["'Plus Jakarta Sans'", SCRIPT[locale] ? `'${SCRIPT[locale]}'` : null, 'sans-serif'].filter(Boolean).join(', ');
  const [first, ...rest] = lines.map(escape);
  return `<!doctype html><html lang="${locale}"><head><meta charset="utf-8"><link rel="stylesheet" href="${FONTS}"><style>
  * { box-sizing: border-box; margin: 0; }
  body { width: 1200px; height: 630px; font-family: ${family}; color: #14284b; background: #fff; overflow: hidden; }
  .card { position: relative; width: 100%; height: 100%; padding: 96px 72px 56px; display: flex; flex-direction: column;
    background: radial-gradient(900px 520px at 100% 100%, #eaf0ff 0%, rgba(234, 240, 255, 0) 70%), #fff; }
  .name { display: flex; align-items: center; gap: 14px; font-family: 'Plus Jakarta Sans', sans-serif; font-weight: 700;
    font-size: 36px; letter-spacing: -0.03em; }
  .name svg { height: 40px; width: auto; }
  .dot { color: #f06445; }
  main { flex: 1; display: flex; flex-direction: column; justify-content: center; max-width: 900px; }
  .label { align-self: flex-start; margin-bottom: 28px; padding: 8px 20px; border-radius: 999px; background: #eaf0ff;
    color: #2457f5; font-size: 28px; font-weight: 700; }
  h1 { font-weight: 700; letter-spacing: -0.03em; line-height: 1.1; }
  h1 span { display: block; }
  h1 .accent { color: #2457f5; }
  .art { position: absolute; right: 64px; bottom: 48px; width: 170px; height: 170px; }
  .art svg { width: 100%; height: 100%; }
  </style></head><body><div class="card">
  <main>${label ? `<div class="label">${escape(label)}</div>` : ''}<h1><span>${first}</span>${rest.map((l) => `<span class="accent">${l}</span>`).join('')}</h1></main>
  <div class="name">${MARK}<span>easi<span class="dot">.</span>run</span></div>
  <div class="art">${MARK}</div>
  </div></body></html>`;
}

/** Shrinks the headline from 84px (never below 60px, to stay legible in a small preview) until it fits in its room,
 * in at most three lines. */
async function fit(page) {
  await page.evaluate(async () => {
    await document.fonts.ready;
    const h1 = document.querySelector('h1');
    const main = document.querySelector('main');
    for (let size = 84; size >= 60; size -= 2) {
      h1.style.fontSize = `${size}px`;
      const lines = Math.round(h1.getBoundingClientRect().height / (size * 1.1));
      if (lines <= 3 && main.scrollHeight <= main.clientHeight && h1.scrollWidth <= h1.clientWidth) return;
    }
  });
}

const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: { width: 1200, height: 630 } });
  const names = {};
  for (const locale of LOCALES) {
    const dir = join(site, 'public', 'og', locale);
    await rm(dir, { recursive: true, force: true });
    await mkdir(dir, { recursive: true });
    names[locale] = {};
    for (const which of ['home', 'features', 'compare']) {
      await page.setContent(html(locale, await card(locale, which)), { waitUntil: 'networkidle' });
      await fit(page);
      const png = await page.screenshot();
      const name = `${which}.${createHash('sha256').update(png).digest('hex').slice(0, 10)}.png`;
      await writeFile(join(dir, name), png);
      names[locale][which] = name;
    }
  }
  await writeFile(join(site, 'src', 'lib', 'cards.json'), `${JSON.stringify(names, null, 2)}\n`);
  // The icon a phone keeps for the site: the mark on white, with room around it.
  const icon = await browser.newPage({ viewport: { width: 180, height: 180 } });
  await icon.setContent(`<body style="margin:0;display:grid;place-items:center;width:180px;height:180px;background:#fff">${MARK.replace('<svg ', '<svg style="height:116px" ')}</body>`);
  await icon.screenshot({ path: join(site, 'public', 'apple-touch-icon.png') });
  // And /favicon.ico, which some crawlers and readers ask for whatever the page names: the mark at 48×48, one PNG in an
  // ICO's header.
  await icon.setViewportSize({ width: 48, height: 48 });
  await icon.setContent(`<body style="margin:0;display:grid;place-items:center;width:48px;height:48px">${MARK.replace('<svg ', '<svg style="height:42px" ')}</body>`);
  const png = await icon.screenshot({ omitBackground: true });
  const head = Buffer.alloc(22);
  head.writeUInt16LE(1, 2); // an icon
  head.writeUInt16LE(1, 4); // one image
  head.writeUInt8(48, 6);
  head.writeUInt8(48, 7);
  head.writeUInt16LE(1, 10); // planes
  head.writeUInt16LE(32, 12); // bits per pixel
  head.writeUInt32LE(png.length, 14);
  head.writeUInt32LE(22, 18);
  await writeFile(join(site, 'public', 'favicon.ico'), Buffer.concat([head, png]));
} finally {
  await browser.close();
}
