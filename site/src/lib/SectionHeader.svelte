<script lang="ts">
  // The one way a section opens: its headline, centred, always the same; a page's own opening one is its h1.
  import type { Snippet } from 'svelte';

  let { children, tag = 'h2' }: { children: Snippet; tag?: 'h1' | 'h2' } = $props();
</script>

<div class="mx-auto max-w-3xl text-center">
  <svelte:element this={tag} class="text-[clamp(2.25rem,3.6vw,3.25rem)] font-semibold leading-[1.05] tracking-[-0.03em]">{@render children()}</svelte:element>
</div>
