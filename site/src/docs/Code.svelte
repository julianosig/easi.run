<script lang="ts">
  // A code block with a copy button, highlighted in its language (`lang`, also its label) when the docs read it.
  import { focus } from '../lib/focus.ts';
  import { highlight } from './highlight.ts';
  import { t } from './i18n.ts';
  let { code, lang = '' }: { code: string; lang?: string } = $props();
  const html = $derived(highlight(code, lang));
  let copied = $state(false);
  let reset: ReturnType<typeof setTimeout> | undefined;

  async function copy() {
    try {
      await navigator.clipboard.writeText(code);
      copied = true;
      // Each copy shows "Copied" for the full two seconds, however soon after the last one it comes.
      clearTimeout(reset);
      reset = setTimeout(() => (copied = false), 2000);
    } catch {
      // Selecting the text by hand still works.
    }
  }
</script>

<div class="group relative mt-5 overflow-hidden rounded-xl bg-void text-[13.5px] leading-6 text-white/90 ring-1 ring-ink/10">
  <div class="flex items-center justify-between border-b border-white/10 px-4 py-2 text-xs font-semibold text-white/50">
    <span>{lang}</span>
    <button
      class="rounded-md px-2 py-1 text-white/70 transition hover:bg-white/10 hover:text-white {focus}"
      type="button"
      onclick={copy}>{copied ? t('shell.copied') : t('shell.copy')}</button
    >
  </div>
  <!-- highlight.js escapes the code: only its own spans are markup. -->
  <pre class="code overflow-x-auto px-4 py-3.5 font-mono"><code>{#if html !== null}{@html html}{:else}{code}{/if}</code></pre>
</div>

<style>
  /* Tokens on the block's dark ground, in the brand's blue and coral, lightened to read on it. */
  .code :global(.hljs-comment),
  .code :global(.hljs-meta) {
    color: rgb(255 255 255 / 0.45);
  }
  .code :global(.hljs-keyword),
  .code :global(.hljs-built_in),
  .code :global(.hljs-literal),
  .code :global(.hljs-selector-tag),
  .code :global(.hljs-name) {
    color: #8fb0ff;
  }
  .code :global(.hljs-string),
  .code :global(.hljs-regexp) {
    color: #9be3b4;
  }
  .code :global(.hljs-attr),
  .code :global(.hljs-attribute),
  .code :global(.hljs-section),
  .code :global(.hljs-title) {
    color: #ffb199;
  }
  .code :global(.hljs-number),
  .code :global(.hljs-variable),
  .code :global(.hljs-template-variable),
  .code :global(.hljs-params) {
    color: #f7d58b;
  }
</style>
