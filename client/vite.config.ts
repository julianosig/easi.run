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

/** What the terminal's WebGL renderer needs for its cells' backgrounds to follow the chat's transparency, whatever color a
 * program paints them. It paints a cell's default background whenever anything is set in the cell's background word, and
 * dim and italic text are flags in that word: it asks the word's color mode instead (`Attributes.CM_MASK`). It draws every
 * background opaque: it takes `__terminalCellOpacity` instead, which the chat sets. And its canvas takes the alpha as it
 * blends it, which a page composes as if squared: it keeps it as it is. The addon's version is pinned, and another one's
 * code stops the build here rather than bring any of it back. */
function clearTerminalCells(): Plugin {
  const CHANGES = [
    ['(u!==0||d&&c!==0)', '(u&50331648||d&&c!==0)', 2],
    ['Vn=1,this._addRectangle(', 'Vn=globalThis.__terminalCellOpacity??1,this._addRectangle(', 1],
    ['r.blendFunc(r.SRC_ALPHA,r.ONE_MINUS_SRC_ALPHA)', 'r.blendFuncSeparate(r.SRC_ALPHA,r.ONE_MINUS_SRC_ALPHA,r.ONE,r.ONE_MINUS_SRC_ALPHA)', 1],
  ] as const;
  return {
    name: 'clear-terminal-cells',
    enforce: 'pre',
    transform(code, id) {
      if (!id.split('?')[0]!.endsWith('/@xterm/addon-webgl/lib/addon-webgl.mjs')) return;
      for (const [asked, instead, times] of CHANGES) {
        if (code.split(asked).length !== times + 1) this.error(`the terminal's renderer no longer has ${asked}`);
        code = code.replaceAll(asked, instead);
      }
      return { code, map: null };
    },
  };
}

// The admin page. Everything is an external file the admin serves itself: its CSP (script-src and style-src 'self')
// allows no inline script, no inline stylesheet and no data: font, so nothing is inlined.
export default defineConfig({
  plugins: [clearTerminalCells(), tailwindcss(), svelte(), messages(fileURLToPath(new URL('./messages/', import.meta.url))), preload()],
  base: './',
  // Served as it is by the dev server too, so `clearTerminalCells` reaches it.
  optimizeDeps: { exclude: ['@xterm/addon-webgl'] },
  // Every built file but the page carries a hash of its contents in its name, under hashed/, which the admin lets
  // browsers keep for a year (admin/src/api.rs, `statics_headers`).
  build: { target: 'es2022', outDir: 'dist', assetsDir: 'hashed', emptyOutDir: true, assetsInlineLimit: 0, cssCodeSplit: false },
});
