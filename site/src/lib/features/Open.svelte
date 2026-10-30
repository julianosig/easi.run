<script lang="ts">
  // Open source, drawn: a file of code, its lines in a few colours, and a magnifying glass that passes over them line
  // by line, the line under it lit. Decorative. It loops every 8 s, moving only transforms and opacity; for less motion
  // it stands still over one line.
  const LINES = [
    { x: 0, w: 60, c: 'fill-accent' },
    { x: 12, w: 96, c: 'fill-ink/30' },
    { x: 12, w: 72, c: 'fill-you/60' },
    { x: 24, w: 84, c: 'fill-ink/30' },
    { x: 12, w: 48, c: 'fill-accent/60' },
    { x: 0, w: 20, c: 'fill-ink/30' },
  ];
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <rect x="64" y="10" width="192" height="108" rx="12" class="fill-paper stroke-line" stroke-width="1.5" />
  <path d="M84 28l-8 8 8 8M236 28l8 8-8 8" class="fill-none stroke-accent/40" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" />
  {#each LINES as line, i (i)}
    <rect x={96 + line.x} y={26 + i * 14} width={line.w} height="6" rx="3" class={line.c} />
  {/each}
  <!-- The glass. -->
  <g class="glass">
    <circle cx="150" cy="40" r="16" class="fill-accent/10 stroke-ink" stroke-width="3" />
    <path d="M161.5 51.5 174 64" class="stroke-ink" stroke-width="4" stroke-linecap="round" />
  </g>
</svg>

<style>
  .glass {
    transform: translate(10px, 28px);
    animation: glass 8s ease-in-out infinite;
  }

  @keyframes glass {
    0%,
    100% {
      transform: translate(-30px, -6px);
    }
    25% {
      transform: translate(30px, 8px);
    }
    50% {
      transform: translate(-10px, 36px);
    }
    75% {
      transform: translate(40px, 56px);
    }
  }
</style>
