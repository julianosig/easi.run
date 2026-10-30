<script lang="ts">
  // The browser window most drawings show, as the agents' card first drew it: a 320×128 box, the window's toolbar and
  // page, and what the drawing puts on the page in front of them. Decorative.
  import type { Snippet } from 'svelte';

  let { children }: { children?: Snippet } = $props();
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <path d="M34 6h252a10 10 0 0 1 10 10v8H24v-8a10 10 0 0 1 10-10z" class="fill-accent-soft" />
  <rect x="24" y="24" width="272" height="98" class="fill-paper" />
  <rect x="24" y="6" width="272" height="116" rx="10" class="fill-none stroke-line" stroke-width="1.5" />
  <circle cx="36" cy="15" r="2.5" class="fill-white" /><circle cx="44" cy="15" r="2.5" class="fill-white" /><circle cx="52" cy="15" r="2.5" class="fill-white" />
  <rect x="64" y="10.5" width="112" height="9" rx="4.5" class="fill-paper" />
  {@render children?.()}
</svg>
