<script lang="ts">
  // The one way a section opens: its headline, centred, always the same.
  import type { Snippet } from 'svelte';

  let { children }: { children: Snippet } = $props();
</script>

<div class="mx-auto max-w-3xl text-center">
  <h2 class="text-[clamp(2.25rem,3.6vw,3.25rem)] font-semibold leading-[1.05] tracking-[-0.03em]">{@render children()}</h2>
</div>
