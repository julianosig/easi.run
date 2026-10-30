<script lang="ts">
  // A fresh Chrome, drawn: a used page (scribbles, a stray extension) is swept clean as a new session starts, a sparkle
  // marks it new, and the locked-down policy (a crossed-out extension piece) stays on its toolbar. Decorative. It loops
  // every 7 s, moving only transforms and opacity; for less motion it stands still, clean.
  import Window from './Window.svelte';
</script>

<Window>
  <defs>
    <clipPath id="fresh-page"><rect x="24" y="24" width="272" height="98" /></clipPath>
  </defs>
  <!-- The clean page. -->
  <rect x="40" y="40" width="72" height="8" rx="4" class="fill-ink/15" />
  <rect x="40" y="56" width="120" height="5" rx="2.5" class="fill-wash" />
  <rect x="40" y="66" width="100" height="5" rx="2.5" class="fill-wash" />
  <g class="sparkle"><path d="M236 70l4 10 10 4-10 4-4 10-4-10-10-4 10-4z" class="fill-accent" /></g>

  <!-- What the last session left: swept away. -->
  <g clip-path="url(#fresh-page)">
    <g class="used">
      <rect x="24" y="24" width="272" height="98" class="fill-paper" />
      <path d="M44 50c20-14 34 10 52-2s30 12 50 0M52 82c24 8 40-12 64 0s36-6 58 2M200 44c12 10 26-6 40 6" class="fill-none stroke-you/50" stroke-width="3" stroke-linecap="round" />
      <rect x="200" y="74" width="64" height="30" rx="6" class="fill-wash stroke-line" stroke-width="1.5" />
      <path d="M220 82h6a3 3 0 0 1 6 0h6v6a3 3 0 0 1 0 6v6h-18z" class="fill-muted/60" />
    </g>
    <rect x="16" y="24" width="10" height="98" class="sweep fill-accent/50" />
  </g>

  <!-- The policy on the toolbar: no extensions. -->
  <g transform="translate(272 15)">
    <path d="M-5 -3h3a2 2 0 0 1 4 0h3v3a2 2 0 0 1 0 4v3h-10z" class="fill-muted" />
    <path d="M-7 7 7 -7" class="stroke-you" stroke-width="2" stroke-linecap="round" />
  </g>
</Window>

<style>
  .used {
    opacity: 0;
    animation: used 7s ease-in-out infinite;
  }
  .sweep {
    opacity: 0;
    animation: sweep 7s ease-in-out infinite;
  }
  .sparkle {
    transform-box: fill-box;
    transform-origin: center;
    animation: sparkle 7s ease-out infinite;
  }

  @keyframes used {
    0%,
    30% {
      transform: none;
      opacity: 1;
    }
    54%,
    100% {
      transform: translateX(272px);
      opacity: 1;
    }
  }
  @keyframes sweep {
    0%,
    30% {
      transform: none;
      opacity: 0;
    }
    32% {
      opacity: 1;
    }
    54% {
      transform: translateX(272px);
      opacity: 1;
    }
    56%,
    100% {
      transform: translateX(272px);
      opacity: 0;
    }
  }
  @keyframes sparkle {
    0%,
    54% {
      transform: scale(0) rotate(-45deg);
    }
    62% {
      transform: scale(1.2) rotate(0);
    }
    68%,
    92% {
      transform: none;
      opacity: 1;
    }
    100% {
      transform: none;
      opacity: 0;
    }
  }
</style>
