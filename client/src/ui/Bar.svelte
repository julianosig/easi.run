<script lang="ts">
  // The top bar, as on the landing: the mark and the name, and on the right whatever the view needs.
  import type { Snippet } from 'svelte';
  import Mark from './Mark.svelte';

  let { children }: { children?: Snippet } = $props();
</script>

<header class="mx-auto flex h-16 max-w-[76rem] items-center justify-between gap-4 rounded-2xl border border-line bg-paper/90 pl-5 pr-3 shadow-[0_10px_34px_-14px_rgb(20_40_75/0.28)] backdrop-blur-md">
  <!-- The cursor stands in the line, its middle level with the middle of the short letters, as on the landing. -->
  <p class="whitespace-nowrap text-[22px] font-semibold tracking-[-0.03em]">
    <Mark class="mr-[0.45em] inline-block h-[1.11em] w-auto align-[-0.28em]" />easi<span class="text-you">.</span>run
  </p>
  <div class="flex items-center gap-2">{@render children?.()}</div>
</header>
