// After the page's build: renders each of the landing's pages with Svelte's server renderer once per language, into
// dist/<page>/index.html (English) and dist/<locale>/<page>/index.html (src/lib/site.ts's PAGES: none for the first
// page), so its words, the comparison and the demo's frame come with the HTML itself, before any script (and for a
// reader that runs none); src/main.ts then hydrates it. And the sitemap of those pages. Each page's link preview card
// (scripts/cards.mjs draws them) must be there.
// The documentation stays rendered in the browser: its pages follow the address's hash.
import { access, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { build } from 'vite';
import { LOCALES, SOURCE } from '../../i18n/locales.ts';
import { readCatalog } from '../../i18n/catalog.ts';
import { pathOf } from '../src/i18n/locale.ts';
import { ORIGIN, PAGES } from '../src/lib/site.ts';
import CARDS from '../src/lib/cards.json' with { type: 'json' };

// Inside the package, so the server build's imports (svelte's own) resolve from its node_modules.
const out = join('node_modules', '.prerender');
try {
  await build({ logLevel: 'warn', build: { ssr: 'src/render.ts', outDir: out, emptyOutDir: true } });
  const { page } = await import(pathToFileURL(join(out, 'render.js')).href);
  const template = await readFile('dist/index.html', 'utf8');
  const empty = '<div id="app"></div>';
  const english = '<html lang="en">';
  if (!template.includes(empty) || !template.includes(english)) throw new Error(`dist/index.html lacks ${empty} or ${english}`);
  for (const locale of LOCALES) {
    const { messages } = readCatalog('messages', locale);
    for (const [which, path] of Object.entries(PAGES)) {
      const { head, body } = page(locale, messages, which);
      const html = template
        .replace(english, `<html lang="${locale}">`)
        .replace('</head>', `${head}</head>`)
        .replace(empty, `<div id="app">${body}</div>`);
      const card = join('dist', 'og', locale, CARDS[locale]?.[which] ?? `${which}.png`);
      await access(card).catch(() => {
        throw new Error(`${card} is missing: draw the cards with scripts/cards.mjs`);
      });
      const dir = join('dist', locale === SOURCE ? '' : locale, path);
      await mkdir(dir, { recursive: true });
      await writeFile(join(dir, 'index.html'), html);
    }
  }
  const urls = Object.values(PAGES)
    .flatMap((path) => LOCALES.map((locale) => `  <url><loc>${ORIGIN}${pathOf(locale, path)}</loc></url>`))
    .join('\n');
  await writeFile('dist/sitemap.xml', `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls}\n</urlset>\n`);
} finally {
  await rm(out, { recursive: true, force: true });
}
