<script lang="ts">
  // The one way a section opens: a small label and the headline, centred, always the same.
  import type { Snippet } from 'svelte';

  let { label, children }: { label: string; children: Snippet } = $props();
</script>

<div class="mx-auto max-w-3xl text-center">
  <p class="text-sm font-semibold uppercase tracking-[0.14em] text-accent">{label}</p>
  <h2 class="mt-4 text-[clamp(2.25rem,3.6vw,3.25rem)] font-semibold leading-[1.05] tracking-[-0.03em]">{@render children()}</h2>
</div>
