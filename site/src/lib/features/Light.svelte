<script lang="ts">
  // Light on bandwidth, drawn: two pipes carry the same scrolling page; the VNC stream's is wide and crowded, mine is a
  // narrow one, both flowing. Decorative. It loops every 2 s, moving only transforms and opacity; for less motion it
  // stands still.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <defs>
    <clipPath id="light-wide"><rect x="96" y="20" width="208" height="44" rx="10" /></clipPath>
    <clipPath id="light-narrow"><rect x="96" y="90" width="208" height="12" rx="6" /></clipPath>
  </defs>
  <!-- The page both carry. -->
  <rect x="16" y="20" width="64" height="88" rx="8" class="fill-paper stroke-line" stroke-width="1.5" />
  <path d="M24 20h48a8 8 0 0 1 8 8v4H16v-4a8 8 0 0 1 8-8z" class="fill-accent-soft" />
  {#each [42, 54, 66, 78, 90] as y (y)}<rect x="26" y={y} width="44" height="5" rx="2.5" class="fill-wash" />{/each}

  <!-- A VNC stream: wide. -->
  <rect x="96" y="20" width="208" height="44" rx="10" class="fill-wash" />
  <g clip-path="url(#light-wide)">
    <g class="flow">
      {#each Array.from({ length: 12 }, (_, i) => i) as i (i)}
        <rect x={80 + i * 24} y="26" width="16" height="32" rx="4" class="fill-muted/35" />
      {/each}
    </g>
  </g>
  <!-- Mine: narrow. -->
  <rect x="96" y="90" width="208" height="12" rx="6" class="fill-accent-soft" />
  <g clip-path="url(#light-narrow)">
    <g class="flow">
      {#each Array.from({ length: 12 }, (_, i) => i) as i (i)}
        <rect x={80 + i * 24} y="93" width="12" height="6" rx="3" class="fill-accent" />
      {/each}
    </g>
  </g>
</svg>

<style>
  .flow {
    animation: flow 2s linear infinite;
  }

  @keyframes flow {
    from {
      transform: none;
    }
    to {
      transform: translateX(24px);
    }
  }
</style>
