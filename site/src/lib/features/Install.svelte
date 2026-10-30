<script lang="ts">
  // Installing with one prompt, drawn: a terminal where your one line is typed, the install's progress fills, and the
  // summary comes back with a check. Decorative. It loops every 7 s, moving only transforms and opacity; for less
  // motion it stands still, installed.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <rect x="40" y="10" width="240" height="108" rx="12" class="fill-void" />
  <circle cx="56" cy="24" r="3" class="fill-white/30" /><circle cx="66" cy="24" r="3" class="fill-white/30" /><circle cx="76" cy="24" r="3" class="fill-white/30" />
  <!-- The one line. -->
  <path d="m56 44 6 4-6 4" class="fill-none stroke-you" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
  <rect x="70" y="45" width="168" height="6" rx="3" class="typed fill-white/80" />
  <!-- The progress. -->
  <rect x="56" y="66" width="208" height="6" rx="3" class="fill-white/15" />
  <rect x="56" y="66" width="208" height="6" rx="3" class="bar fill-accent" />
  <!-- The summary, and its check. -->
  <g class="done">
    <circle cx="62" cy="92" r="7" class="fill-accent" />
    <path d="m58.5 92 2.5 2.5 4.5-5" class="fill-none stroke-white" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
    <rect x="76" y="89" width="96" height="6" rx="3" class="fill-white/60" />
    <rect x="56" y="104" width="140" height="5" rx="2.5" class="fill-white/25" />
  </g>
</svg>

<style>
  .typed {
    transform-box: fill-box;
    transform-origin: left center;
    animation: typed 7s steps(14, end) infinite;
  }
  .bar {
    transform-box: fill-box;
    transform-origin: left center;
    animation: bar 7s ease-in-out infinite;
  }
  .done {
    animation: done 7s ease-out infinite;
  }

  @keyframes typed {
    0%,
    4% {
      transform: scaleX(0);
    }
    26%,
    100% {
      transform: none;
    }
  }
  @keyframes bar {
    0%,
    30% {
      transform: scaleX(0);
    }
    64%,
    100% {
      transform: none;
    }
  }
  @keyframes done {
    0%,
    66% {
      transform: translateY(4px);
      opacity: 0;
    }
    72%,
    92% {
      transform: none;
      opacity: 1;
    }
    98%,
    100% {
      opacity: 0;
    }
  }
</style>
