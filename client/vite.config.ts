import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

// The admin page. Everything is an external file the admin serves itself: its CSP (script-src and style-src 'self')
// allows no inline script, no inline stylesheet and no data: font, so nothing is inlined.
export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  base: './',
  build: { target: 'es2022', outDir: 'dist', emptyOutDir: true, assetsInlineLimit: 0, cssCodeSplit: false },
});
