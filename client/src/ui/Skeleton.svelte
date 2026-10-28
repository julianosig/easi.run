<script lang="ts">
  // A list on its way: its rows' shapes (badge, name, details, action) in Loading's shimmer, where the rows will be.
  // They fade in only if the answer is slow, so one that comes within a round trip shows no flash. Announced to screen
  // readers.
  import { row, rows as list } from './admin.ts';

  let { said, rows = 3 }: { said: string; rows?: number } = $props();
</script>

<div class="skeleton mt-10" role="status">
  <span class="sr-only">{said}</span>
  <ul class={list} aria-hidden="true">
    {#each { length: rows }, i (i)}
      <li class="flex items-center gap-4 py-3 {row} hover:bg-transparent">
        <span class="shimmer relative size-10 shrink-0 rounded-xl"></span>
        <span class="min-w-0 flex-1 space-y-2.5">
          <span class="shimmer relative block h-3.5 rounded-full" style:width="{[38, 30, 44][i % 3]}%"></span>
          <span class="shimmer relative block h-3 rounded-full" style:width="{[58, 50, 64][i % 3]}%"></span>
        </span>
        <span class="shimmer relative hidden h-9 w-24 rounded-xl sm:block"></span>
      </li>
    {/each}
  </ul>
</div>

<style>
  .skeleton {
    opacity: 0;
    animation: appear 0.2s ease-out 0.3s forwards;
  }
  @keyframes appear {
    to {
      opacity: 1;
    }
  }
</style>
