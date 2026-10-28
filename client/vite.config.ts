import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';
import { defineConfig, type Plugin } from 'vite';
import { messages } from '../i18n/vite.ts';

/** The administrator's views' file, named in `dist/preload`: the admin sends it with the page outside the demo (whose
 * visitors never load it), so it comes along with the page's script instead of after the admin's first answer
 * (admin/src/api.rs, `preload`): some 185 ms less at 215 ms away, over HTTP/2. */
function preload(): Plugin {
  return {
    name: 'preload',
    apply: 'build',
    generateBundle(_, bundle) {
      const views = Object.values(bundle).find((file) => file.type === 'chunk' && file.facadeModuleId?.endsWith('/src/ui/views.ts'));
      if (!views) this.error('no chunk for src/ui/views.ts');
      this.emitFile({ type: 'asset', fileName: 'preload', source: views.fileName });
    },
  };
}

// The admin page. Everything is an external file the admin serves itself: its CSP (script-src and style-src 'self')
// allows no inline script, no inline stylesheet and no data: font, so nothing is inlined.
export default defineConfig({
  plugins: [tailwindcss(), svelte(), messages(fileURLToPath(new URL('./messages/', import.meta.url))), preload()],
  base: './',
  // Every built file but the page carries a hash of its contents in its name, under hashed/, which the admin lets
  // browsers keep for a year (admin/src/api.rs, `statics_headers`).
  build: { target: 'es2022', outDir: 'dist', assetsDir: 'hashed', emptyOutDir: true, assetsInlineLimit: 0, cssCodeSplit: false },
});
