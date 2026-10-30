<script lang="ts">
  // Watching together, drawn: three windows on one session, each with the same page; control (the coral ring and
  // cursor) passes from one window to the next while the others watch. Decorative. It loops every 9 s, moving only
  // transforms and opacity; for less motion it stands still with the first window at the wheel.
  import Cursor from './Cursor.svelte';

  const AT = [16, 116, 216];
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  {#each AT as x, i (x)}
    <rect x={x - 3} y="25" width="94" height="78" rx="11" class="held h{i} fill-none stroke-you" stroke-width="2.5" />
    <rect x={x} y="28" width="88" height="72" rx="9" class="fill-paper stroke-line" stroke-width="1.5" />
    <path d="M{x + 9} 28h70a9 9 0 0 1 9 9v4H{x}v-4a9 9 0 0 1 9-9z" class="fill-accent-soft" />
    <rect x={x + 10} y="50" width="44" height="6" rx="3" class="fill-ink/15" />
    <rect x={x + 10} y="62" width="66" height="4" rx="2" class="fill-wash" />
    <rect x={x + 10} y="80" width="30" height="10" rx="5" class="fill-accent" />
    <!-- Watching: an eye on each window. -->
    <path d="M{x + 62} 85q8-7 16 0q-8 7-16 0z" class="fill-none stroke-muted" stroke-width="1.5" />
    <circle cx={x + 70} cy="85" r="2" class="fill-muted" />
  {/each}
  <g class="you"><Cursor you /></g>
</svg>

<style>
  .held {
    opacity: 0;
    animation: held 9s step-end infinite;
  }
  .h0 {
    opacity: 1;
    animation-name: held-0;
  }
  .h1 {
    animation-name: held-1;
  }
  .h2 {
    animation-name: held-2;
  }
  .you {
    transform: translate(56px, 70px);
    animation: you 9s ease-in-out infinite;
  }

  @keyframes held-0 {
    0% {
      opacity: 1;
    }
    33% {
      opacity: 0;
    }
  }
  @keyframes held-1 {
    0% {
      opacity: 0;
    }
    33% {
      opacity: 1;
    }
    66% {
      opacity: 0;
    }
  }
  @keyframes held-2 {
    0% {
      opacity: 0;
    }
    66% {
      opacity: 1;
    }
  }
  @keyframes you {
    0%,
    26% {
      transform: translate(56px, 70px);
    }
    33%,
    59% {
      transform: translate(156px, 70px);
    }
    66%,
    92% {
      transform: translate(256px, 70px);
    }
    100% {
      transform: translate(56px, 70px);
    }
  }
</style>
