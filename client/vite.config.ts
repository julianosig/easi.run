import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { readdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { defineConfig, type Plugin } from 'vite';

const LANGUAGE = 'virtual:messages/';

/** Each language as one module, `virtual:messages/<locale>`: its catalog with every area's file merged (`area.name`
 * keys), so a page loads its language in one file. The admin speaks HTTP/1.1: a file per area, past the browser's few
 * connections, costs new connections' round trips before the page can show (some 850 ms at 215 ms away). */
function messages(): Plugin {
  const root = fileURLToPath(new URL('./messages/', import.meta.url));
  return {
    name: 'messages',
    resolveId: (id) => (id.startsWith(LANGUAGE) ? `\0${id}` : undefined),
    // A file added or removed changes its language's module too (an edited one is watched by `load`).
    configureServer(server) {
      const changed = (path: string) => {
        const locale = /messages[\\/]([^\\/]+)[\\/][^\\/]+\.json$/.exec(path)?.[1];
        const module = locale === undefined ? undefined : server.moduleGraph.getModuleById(`\0${LANGUAGE}${locale}`);
        if (!module) return;
        server.moduleGraph.invalidateModule(module);
        server.ws.send({ type: 'full-reload' });
      };
      server.watcher.on('add', changed).on('unlink', changed);
    },
    load(id) {
      if (!id.startsWith(`\0${LANGUAGE}`)) return undefined;
      const locale = id.slice(LANGUAGE.length + 1);
      if (!/^[a-z]{2}(-[A-Z]{2})?$/.test(locale)) this.error(`not a language: ${locale}`);
      const catalog: Record<string, unknown> = {};
      for (const file of readdirSync(`${root}${locale}`).filter((f) => f.endsWith('.json')).sort()) {
        const path = `${root}${locale}/${file}`;
        this.addWatchFile(path);
        for (const [name, message] of Object.entries(JSON.parse(readFileSync(path, 'utf8')) as Record<string, unknown>)) catalog[`${file.slice(0, -'.json'.length)}.${name}`] = message;
      }
      return `export default JSON.parse(${JSON.stringify(JSON.stringify(catalog))});`;
    },
  };
}

// The admin page. Everything is an external file the admin serves itself: its CSP (script-src and style-src 'self')
// allows no inline script, no inline stylesheet and no data: font, so nothing is inlined.
export default defineConfig({
  plugins: [tailwindcss(), svelte(), messages()],
  base: './',
  // Every built file but the page carries a hash of its contents in its name, under hashed/, which the admin lets
  // browsers keep for a year (admin/src/api.rs, `statics_headers`).
  build: { target: 'es2022', outDir: 'dist', assetsDir: 'hashed', emptyOutDir: true, assetsInlineLimit: 0, cssCodeSplit: false },
});
