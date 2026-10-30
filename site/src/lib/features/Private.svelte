<script lang="ts">
  // Your network off-limits, drawn: the session's browser in the middle sends requests out; those to the public web (the
  // globe) get through, each with a check, while one to your private network (the house) and one to your server's own
  // services (the server) stop at a barrier. Decorative. It loops every 8 s, moving only transforms and opacity; for
  // less motion it stands still with the barriers up.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The browser. -->
  <rect x="124" y="40" width="72" height="48" rx="8" class="fill-paper stroke-line" stroke-width="1.5" />
  <path d="M132 40h56a8 8 0 0 1 8 8v4h-72v-4a8 8 0 0 1 8-8z" class="fill-accent-soft" />
  <rect x="134" y="62" width="40" height="5" rx="2.5" class="fill-ink/15" />
  <rect x="134" y="72" width="52" height="4" rx="2" class="fill-wash" />

  <!-- The public web, to the right. -->
  <g transform="translate(270 64)" class="fill-none stroke-accent" stroke-width="2.5">
    <circle r="22" />
    <path d="M-22 0h44M0 -22c7 6 10 13 10 22s-3 16-10 22c-7-6-10-13-10-22s3-16 10-22z" />
  </g>
  <path d="M200 64h44" class="fill-none stroke-accent/30" stroke-width="2" stroke-dasharray="4 4" />
  <circle cx="200" cy="64" r="4" class="go fill-accent" />

  <!-- Your private network (a house) and your server (a box), to the left, behind barriers. -->
  <path d="M30 40l18-14 18 14v18H30z" class="fill-wash stroke-ink" stroke-width="2" stroke-linejoin="round" />
  <rect x="30" y="76" width="36" height="28" rx="4" class="fill-wash stroke-ink" stroke-width="2" />
  <path d="M38 84h20M38 92h20" class="stroke-ink/40" stroke-width="2" stroke-linecap="round" />
  <path d="M120 54L74 44M120 74l-46 16" class="fill-none stroke-ink/20" stroke-width="2" stroke-dasharray="4 4" />
  <rect x="86" y="30" width="5" height="70" rx="2.5" class="bar fill-you" />
  <circle cx="120" cy="54" r="4" class="no fill-you" />
  <circle cx="120" cy="74" r="4" class="no later fill-you" />
</svg>

<style>
  .go {
    opacity: 0;
    animation: go 8s ease-in-out infinite;
  }
  .no {
    opacity: 0;
    animation: no 8s ease-in infinite;
  }
  .later {
    animation-delay: 1.6s;
  }
  .bar {
    transform-box: fill-box;
    transform-origin: 50% 100%;
    animation: bar 8s ease-out infinite;
  }

  @keyframes go {
    0%,
    4% {
      transform: none;
      opacity: 0;
    }
    8% {
      opacity: 1;
    }
    26% {
      transform: translateX(42px);
      opacity: 1;
    }
    30%,
    100% {
      transform: translateX(42px);
      opacity: 0;
    }
  }
  @keyframes no {
    0%,
    34% {
      transform: none;
      opacity: 0;
    }
    38% {
      opacity: 1;
    }
    50% {
      transform: translateX(-28px);
      opacity: 1;
    }
    58%,
    100% {
      transform: translateX(-28px);
      opacity: 0;
    }
  }
  @keyframes bar {
    0%,
    44%,
    100% {
      transform: none;
    }
    49% {
      transform: scaleY(1.06);
    }
  }
</style>
