<script lang="ts">
  // Fitting your screen, drawn: a monitor, and the session's browser inside it growing to its shape; then the corners
  // spread out as it goes full screen. Decorative. It loops every 7 s, moving only transforms and opacity; for less
  // motion it stands still filling the screen.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The monitor. -->
  <rect x="70" y="8" width="180" height="104" rx="8" class="fill-ink" />
  <rect x="76" y="14" width="168" height="92" rx="4" class="fill-wash" />
  <path d="M146 112h28l4 10h-36z" class="fill-ink" />
  <!-- The session, fitted. -->
  <g class="fit">
    <rect x="76" y="14" width="168" height="92" rx="4" class="fill-paper" />
    <path d="M80 14h160a4 4 0 0 1 4 4v6H76v-6a4 4 0 0 1 4-4z" class="fill-accent-soft" />
    <rect x="88" y="34" width="64" height="7" rx="3.5" class="fill-ink/15" />
    <rect x="88" y="48" width="120" height="5" rx="2.5" class="fill-wash" />
    <rect x="88" y="58" width="100" height="5" rx="2.5" class="fill-wash" />
    <rect x="88" y="78" width="40" height="12" rx="6" class="fill-accent" />
  </g>
  <!-- Full screen: four corners. -->
  {#each [{ x: 60, y: 6, d: 'M0 10V0h10' }, { x: 260, y: 6, d: 'M0 0h10v10', dx: -10 }, { x: 60, y: 114, d: 'M0 0v10h10', dy: -10 }, { x: 260, y: 114, d: 'M10 0v10H0', dx: -10, dy: -10 }] as c, i (i)}
    <g transform="translate({c.x + (c.dx ?? 0)} {c.y + (c.dy ?? 0)})"><path d={c.d} class="corner q{i} fill-none stroke-accent" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" /></g>
  {/each}
</svg>

<style>
  .fit {
    transform-origin: 160px 60px;
    animation: fit 7s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  }
  .corner {
    opacity: 0;
    animation: corner 7s ease-out infinite;
  }
  .q0 {
    --x: -4px;
    --y: -4px;
  }
  .q1 {
    --x: 4px;
    --y: -4px;
  }
  .q2 {
    --x: -4px;
    --y: 4px;
  }
  .q3 {
    --x: 4px;
    --y: 4px;
  }

  @keyframes fit {
    0%,
    10% {
      transform: scale(0.62, 0.7);
      opacity: 0.6;
    }
    40%,
    92% {
      transform: none;
      opacity: 1;
    }
    100% {
      transform: scale(0.62, 0.7);
      opacity: 0.6;
    }
  }
  @keyframes corner {
    0%,
    48% {
      transform: none;
      opacity: 0;
    }
    56% {
      opacity: 1;
    }
    70% {
      transform: translate(var(--x), var(--y));
      opacity: 0;
    }
    100% {
      opacity: 0;
    }
  }
</style>
