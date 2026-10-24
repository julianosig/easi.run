import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

// Three pages: the landing (easi.run), the documentation (docs.easi.run, served from /docs/), and the logo proposals
// (/logos/, unlisted), sharing one asset pool.
export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  build: {
    target: 'es2022',
    rollupOptions: { input: { main: 'index.html', docs: 'docs/index.html', logos: 'logos/index.html' } },
  },
});
