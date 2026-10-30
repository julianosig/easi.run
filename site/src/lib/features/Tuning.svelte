<script lang="ts">
  // Tuning itself, drawn: three sliders (TCP's pacing, sessions to memory, the UDP path) whose knobs slide to their
  // places one after another, and a gauge whose needle settles in the good zone. Decorative. It loops every 8 s, moving
  // only transforms and opacity; for less motion it stands still, tuned.
  const ROWS = [
    { y: 34, to: 52 },
    { y: 64, to: 30 },
    { y: 94, to: 66 },
  ];
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  {#each ROWS as row, i (row.y)}
    <rect x="28" y={row.y - 2} width="132" height="4" rx="2" class="fill-line" />
    <g class="knob k{i}">
      <circle cx={60 + row.to} cy={row.y} r="8" class="fill-paper stroke-accent" stroke-width="2.5" />
    </g>
  {/each}

  <!-- The gauge. -->
  <path d="M196 96a44 44 0 0 1 88 0" class="fill-none stroke-line" stroke-width="8" stroke-linecap="round" />
  <path d="M254 56a44 44 0 0 1 30 40" class="fill-none stroke-accent" stroke-width="8" stroke-linecap="round" />
  <path d="M240 96l26-24" class="needle fill-none stroke-ink" stroke-width="3.5" stroke-linecap="round" />
  <circle cx="240" cy="96" r="6" class="fill-ink" />
</svg>

<style>
  .knob {
    animation: knob 8s cubic-bezier(0.45, 0, 0.25, 1) infinite;
  }
  /* How far each knob travels to its place: ROWS's `to`. */
  .k0 {
    --to: 52px;
  }
  .k1 {
    --to: 30px;
    animation-delay: 0.6s;
  }
  .k2 {
    --to: 66px;
    animation-delay: 1.2s;
  }
  .needle {
    transform-origin: 240px 96px;
    animation: needle 8s cubic-bezier(0.45, 0, 0.25, 1.3) infinite;
  }

  @keyframes knob {
    0%,
    6% {
      transform: translateX(calc(-1 * var(--to)));
    }
    22%,
    88% {
      transform: none;
    }
    100% {
      transform: translateX(calc(-1 * var(--to)));
    }
  }
  @keyframes needle {
    0%,
    30% {
      transform: rotate(-90deg);
    }
    48%,
    88% {
      transform: none;
    }
    100% {
      transform: rotate(-90deg);
    }
  }
</style>
