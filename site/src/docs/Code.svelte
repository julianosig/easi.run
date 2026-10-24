<script lang="ts">
  // A code block with a copy button. `lang` is only a label.
  import { focus } from '../lib/focus.ts';
  let { code, lang = '' }: { code: string; lang?: string } = $props();
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
      onclick={copy}>{copied ? 'Copied ✓' : 'Copy'}</button
    >
  </div>
  <pre class="overflow-x-auto px-4 py-3.5 font-mono"><code>{code}</code></pre>
</div>
