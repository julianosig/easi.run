// After the page's build: renders the landing with Svelte's server renderer once per language, into dist/index.html
// (English) and dist/<locale>/index.html, so its words, the comparison and the demo's frame come with the HTML itself,
// before any script (and for a reader that runs none); src/main.ts then hydrates it. And the sitemap of those pages.
// The documentation stays rendered in the browser: its pages follow the address's hash.
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { build } from 'vite';
import { LOCALES, SOURCE } from '../../i18n/locales.ts';
import { catalog } from '../../i18n/vite.ts';

const ORIGIN = 'https://easi.run';
const pathOf = (locale) => (locale === SOURCE ? '/' : `/${locale}/`);

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
    const { head, body } = page(locale, catalog('messages', locale).messages);
    const html = template
      .replace(english, `<html lang="${locale}">`)
      .replace('</head>', `${head}</head>`)
      .replace(empty, `<div id="app">${body}</div>`);
    const dir = locale === SOURCE ? 'dist' : join('dist', locale);
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, 'index.html'), html);
  }
  const urls = LOCALES.map((locale) => `  <url><loc>${ORIGIN}${pathOf(locale)}</loc></url>`).join('\n');
  await writeFile('dist/sitemap.xml', `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls}\n</urlset>\n`);
} finally {
  await rm(out, { recursive: true, force: true });
}
