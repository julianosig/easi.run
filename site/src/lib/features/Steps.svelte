<script lang="ts">
  // Many steps in one call, drawn: one call (the bracket) holds a list of steps (a click, typing, a wait, a check), and
  // each is ticked off in turn, quickly, before the call answers once. Decorative. It loops every 7 s, moving only
  // transforms and opacity; for less motion it stands still with every step ticked.
  const STEPS = [
    { y: 18, d: 'M0 -4l8 4-3.5 1L3 4.5z', w: 92 },
    { y: 46, d: 'M-4 -3h8M0 -3v7', w: 112 },
    { y: 74, d: 'M0 -4a4 4 0 1 0 0.01 0M0 -2v2.4l1.6 1.2', w: 80 },
    { y: 102, d: 'M-4 0l3 3 5-6', w: 100 },
  ];
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The call: one bracket around every step. -->
  <path d="M76 8h-10a6 6 0 0 0-6 6v100a6 6 0 0 0 6 6h10" class="fill-none stroke-accent" stroke-width="2.5" stroke-linecap="round" />
  {#each STEPS as step, i (i)}
    <g transform="translate(0 {step.y})">
      <rect x="84" y="-11" width="176" height="22" rx="11" class="fill-paper stroke-line" stroke-width="1.5" />
      <g transform="translate(100 0)"><path d={step.d} class="fill-none stroke-muted" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></g>
      <rect x="116" y="-2.5" width={step.w} height="5" rx="2.5" class="fill-ink/15" />
      <g class="tick t{i}" transform="translate(246 0)">
        <circle r="7" class="fill-accent" />
        <path d="m-3 0 2.2 2.2 4-4.4" class="fill-none stroke-white" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
      </g>
    </g>
  {/each}
  <!-- The single answer. -->
  <circle cx="282" cy="64" r="6" class="answer fill-you" />
</svg>

<style>
  .tick > * {
    transform-box: fill-box;
    transform-origin: center;
    animation: tick 7s ease-out infinite;
  }
  .t1 > * {
    animation-delay: 0.45s;
  }
  .t2 > * {
    animation-delay: 0.9s;
  }
  .t3 > * {
    animation-delay: 1.35s;
  }
  .answer {
    transform-box: fill-box;
    transform-origin: center;
    animation: answer 7s ease-out infinite;
  }

  @keyframes tick {
    0%,
    12% {
      transform: scale(0);
      opacity: 0;
    }
    17% {
      transform: scale(1.2);
      opacity: 1;
    }
    20%,
    86% {
      transform: scale(1);
      opacity: 1;
    }
    94%,
    100% {
      transform: scale(1);
      opacity: 0;
    }
  }
  @keyframes answer {
    0%,
    38% {
      transform: scale(0);
      opacity: 0;
    }
    44% {
      transform: scale(1.3);
      opacity: 1;
    }
    50%,
    90% {
      transform: scale(1);
      opacity: 1;
    }
    98%,
    100% {
      opacity: 0;
    }
  }
</style>
