// After the page's build: renders the landing with Svelte's server renderer into dist/index.html, so its words, the
// comparison and the demo's frame come with the HTML itself, before any script (and for a reader that runs none);
// src/main.ts then hydrates it. The documentation stays rendered in the browser: its pages follow the address's hash.
import { readFile, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { render } from 'svelte/server';
import { build } from 'vite';

// Inside the package, so the server build's imports (svelte's own) resolve from its node_modules.
const out = join('node_modules', '.prerender');
try {
  await build({ logLevel: 'warn', build: { ssr: 'src/App.svelte', outDir: out, emptyOutDir: true } });
  const { default: App } = await import(pathToFileURL(join(out, 'App.js')).href);
  const { head, body } = render(App);
  const page = await readFile('dist/index.html', 'utf8');
  const empty = '<div id="app"></div>';
  if (!page.includes(empty)) throw new Error(`dist/index.html has no ${empty}`);
  await writeFile('dist/index.html', page.replace('</head>', `${head}</head>`).replace(empty, `<div id="app">${body}</div>`));
} finally {
  await rm(out, { recursive: true, force: true });
}
