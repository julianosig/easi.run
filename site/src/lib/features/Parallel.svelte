<script lang="ts">
  // Several at once, drawn: three browsers open one after another, side by side, and an agent works in each on its own
  // path. Decorative. It loops every 8 s, moving only transforms and opacity; for less motion it stands still with the
  // three open, an agent in each.
  import Cursor from './Cursor.svelte';

  const AT = [16, 116, 216];
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  {#each AT as x, i (x)}
    <g class="window w{i}">
      <rect x={x} y="22" width="88" height="84" rx="9" class="fill-paper stroke-line" stroke-width="1.5" />
      <path d="M{x + 9} 22h70a9 9 0 0 1 9 9v5H{x}v-5a9 9 0 0 1 9-9z" class="fill-accent-soft" />
      <rect x={x + 10} y="46" width="44" height="6" rx="3" class="fill-ink/15" />
      <rect x={x + 10} y="58" width="66" height="4" rx="2" class="fill-wash" />
      <rect x={x + 10} y="67" width="56" height="4" rx="2" class="fill-wash" />
      <rect x={x + 10} y="84" width="30" height="10" rx="5" class="fill-accent" />
      <g transform="translate({x} 0)"><g class="agent a{i}"><Cursor /></g></g>
    </g>
  {/each}
</svg>

<style>
  .window {
    animation: open 8s ease-out infinite;
  }
  .w1 {
    animation-name: open-1;
  }
  .w2 {
    animation-name: open-2;
  }
  .agent {
    transform: translate(30px, 88px);
    animation: work 4s ease-in-out infinite;
  }
  .a1 {
    animation-duration: 5s;
    animation-direction: reverse;
  }
  .a2 {
    animation-duration: 4.5s;
  }

  @keyframes open {
    0%,
    4% {
      transform: translateY(6px);
      opacity: 0;
    }
    10%,
    90% {
      transform: none;
      opacity: 1;
    }
    97%,
    100% {
      opacity: 0;
    }
  }
  @keyframes open-1 {
    0%,
    16% {
      transform: translateY(6px);
      opacity: 0;
    }
    22%,
    90% {
      transform: none;
      opacity: 1;
    }
    97%,
    100% {
      opacity: 0;
    }
  }
  @keyframes open-2 {
    0%,
    28% {
      transform: translateY(6px);
      opacity: 0;
    }
    34%,
    90% {
      transform: none;
      opacity: 1;
    }
    97%,
    100% {
      opacity: 0;
    }
  }
  @keyframes work {
    0%,
    100% {
      transform: translate(30px, 88px);
    }
    33% {
      transform: translate(62px, 60px);
    }
    66% {
      transform: translate(40px, 50px);
    }
  }
</style>
