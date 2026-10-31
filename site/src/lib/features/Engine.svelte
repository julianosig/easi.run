<script lang="ts">
  // Our own streaming engine, drawn: the page arrives as tiles, each sharpening into place; then a stretch of motion
  // passes as one smooth stream; and once it stops, those tiles sharpen again. Decorative. It loops every 8 s, moving
  // only transforms and opacity; for less motion it stands still with every tile sharp.
  import Window from './Window.svelte';

  const COLS = 8;
  const ROWS = 4;
  const tiles = Array.from({ length: COLS * ROWS }, (_, i) => ({ x: 36 + (i % COLS) * 31, y: 32 + Math.floor(i / COLS) * 21, i }));
</script>

<Window>
  {#each tiles as tile (tile.i)}
    <g class="tile" style="animation-delay: {(tile.i % COLS) * 0.06 + Math.floor(tile.i / COLS) * 0.12}s">
      <rect x={tile.x} y={tile.y} width="28" height="18" rx="3" class={tile.i % 5 === 0 ? 'fill-accent-soft' : 'fill-wash'} />
      <rect x={tile.x + 4} y={tile.y + 7} width={tile.i % 3 === 0 ? 16 : 20} height="4" rx="2" class="fill-ink/25" />
    </g>
  {/each}
  <!-- Motion, as one smooth stream over a band of the page. -->
  <g class="motion">
    <rect x="36" y="74" width="245" height="18" rx="3" class="fill-accent/15" />
    <path d="M40 83h20m10 0h40m10 0h30m10 0h60m10 0h40" class="fill-none stroke-accent" stroke-width="3" stroke-linecap="round" />
  </g>
</Window>

<style>
  .tile {
    animation: tile 8s ease-out infinite;
  }
  .motion {
    opacity: 0;
    animation: motion 8s ease-in-out infinite;
  }

  @keyframes tile {
    0% {
      opacity: 0;
      transform: translateY(4px);
    }
    10%,
    88% {
      opacity: 1;
      transform: none;
    }
    96%,
    100% {
      opacity: 0;
    }
  }
  @keyframes motion {
    0%,
    40% {
      opacity: 0;
      transform: translateX(-40px);
    }
    48% {
      opacity: 1;
    }
    66% {
      opacity: 1;
      transform: translateX(40px);
    }
    72%,
    100% {
      opacity: 0;
      transform: translateX(40px);
    }
  }
</style>
