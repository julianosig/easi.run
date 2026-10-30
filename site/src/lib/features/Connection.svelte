<script lang="ts">
  // The connection panel, drawn: a card with the link's grade (four bars filling up), a live line of the frame rate
  // running past, and rows of figures. Decorative. It loops every 6 s, moving only transforms and opacity; for less
  // motion it stands still on an excellent link.

  /** The frame rate's line: one 104-unit stretch, repeated, so its loop never shows a seam. */
  const SHAPE = [52, 46, 50, 42, 48, 44, 51, 46];
  const LINE = 'M150 52' + Array.from({ length: 24 }, (_, i) => `L${150 + (i + 1) * 13} ${SHAPE[(i + 1) % 8]}`).join('');
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <defs>
    <clipPath id="connection-graph"><rect x="150" y="26" width="130" height="44" rx="6" /></clipPath>
  </defs>
  <rect x="24" y="12" width="272" height="104" rx="12" class="fill-paper stroke-line" stroke-width="1.5" />
  <!-- The grade. -->
  {#each [0, 1, 2, 3] as i (i)}
    <rect x={44 + i * 14} y={60 - i * 10} width="9" height={14 + i * 10} rx="3" class="bar b{i} fill-accent" />
  {/each}
  <rect x="44" y="84" width="60" height="6" rx="3" class="fill-ink/25" />
  <rect x="44" y="96" width="40" height="5" rx="2.5" class="fill-wash" />

  <!-- The live line. -->
  <rect x="150" y="26" width="130" height="44" rx="6" class="fill-wash" />
  <g clip-path="url(#connection-graph)">
    <path d={LINE} class="line fill-none stroke-accent" stroke-width="2" stroke-linejoin="round" />
  </g>
  <!-- Figures. -->
  {#each [82, 96] as y (y)}
    <rect x="150" y={y} width="44" height="5" rx="2.5" class="fill-ink/15" />
    <rect x="236" y={y} width="44" height="5" rx="2.5" class="fill-ink/30" />
  {/each}
</svg>

<style>
  .bar {
    transform-box: fill-box;
    transform-origin: 50% 100%;
    animation: bar 6s ease-out infinite;
  }
  .b1 {
    animation-delay: 0.2s;
  }
  .b2 {
    animation-delay: 0.4s;
  }
  .b3 {
    animation-delay: 0.6s;
  }
  .line {
    animation: line 6s linear infinite;
  }

  @keyframes bar {
    0%,
    6% {
      transform: scaleY(0);
    }
    16%,
    90% {
      transform: none;
    }
    100% {
      transform: scaleY(0);
    }
  }
  @keyframes line {
    from {
      transform: none;
    }
    to {
      transform: translateX(-104px);
    }
  }
</style>
