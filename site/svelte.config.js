import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

// Every component in runes mode: one without a rune would otherwise compile, and load the runtime, in legacy mode.
export default { preprocess: vitePreprocess(), compilerOptions: { runes: true } };
