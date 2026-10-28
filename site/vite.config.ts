import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';
import { messages } from '../i18n/vite.ts';

// Two pages, the landing (easi.run) and the documentation (docs.easi.run, served from /docs/), sharing one asset pool.
export default defineConfig({
  plugins: [tailwindcss(), svelte(), messages('messages'), messages('src/docs/messages', 'virtual:docs-messages/')],
  build: {
    target: 'es2022',
    rollupOptions: { input: { main: 'index.html', docs: 'docs/index.html' } },
  },
});
