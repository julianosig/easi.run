<script lang="ts">
  // A short aside: `tone` "info" for context, "warn" for something that changes a guarantee.
  import type { Snippet } from 'svelte';
  let { tone = 'info', children }: { tone?: 'info' | 'warn'; children: Snippet } = $props();
</script>

<div class="mt-5 rounded-xl border px-4 py-3 text-[15px] {tone === 'warn' ? 'border-partly/30 bg-[light-dark(#fff7ed,#2b2013)]' : 'border-accent/20 bg-accent-soft'}">
  {@render children()}
</div>
