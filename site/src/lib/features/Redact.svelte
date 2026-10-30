<script lang="ts">
  // What you type stays yours, drawn: on the left you type a password into the page (your coral cursor, the dots
  // appearing); on the right, what the agent then reads: the same field, its text struck out as [redacted]. Decorative.
  // It loops every 8 s, moving only transforms and opacity; for less motion it stands still, typed and redacted.
  import Cursor from './Cursor.svelte';
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <defs>
    <pattern id="redact-stripes" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
      <rect width="3" height="6" class="fill-ink" />
    </pattern>
  </defs>
  <!-- You, typing. -->
  <rect x="16" y="28" width="132" height="72" rx="10" class="fill-paper stroke-you/60" stroke-width="1.5" />
  <rect x="28" y="42" width="44" height="5" rx="2.5" class="fill-ink/15" />
  <rect x="28" y="54" width="108" height="22" rx="6" class="fill-paper stroke-line" stroke-width="1.5" />
  {#each [0, 1, 2, 3, 4, 5, 6] as i (i)}<circle cx={40 + i * 12} cy="65" r="3" class="dot d{i} fill-ink" />{/each}
  <g transform="translate(120 72)"><Cursor you /></g>

  <!-- The arrow to the agent. -->
  <path d="M156 64h20m-6-6 6 6-6 6" class="fill-none stroke-muted" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />

  <!-- What the agent reads. -->
  <rect x="184" y="28" width="120" height="72" rx="10" class="fill-wash stroke-accent/60" stroke-width="1.5" />
  <rect x="196" y="42" width="44" height="5" rx="2.5" class="fill-ink/15" />
  <rect x="196" y="54" width="96" height="22" rx="6" class="fill-paper stroke-line" stroke-width="1.5" />
  <g class="redacted">
    <rect x="204" y="60" width="80" height="10" rx="2" class="fill-ink" />
    <rect x="204" y="60" width="80" height="10" rx="2" fill="url(#redact-stripes)" class="opacity-40" />
  </g>
  <g transform="translate(286 92)"><Cursor /></g>
</svg>

<style>
  .dot {
    animation: dot 8s step-end infinite;
  }
  .d1 {
    animation-name: dot-1;
  }
  .d2 {
    animation-name: dot-2;
  }
  .d3 {
    animation-name: dot-3;
  }
  .d4 {
    animation-name: dot-4;
  }
  .d5 {
    animation-name: dot-5;
  }
  .d6 {
    animation-name: dot-6;
  }
  .redacted {
    transform-box: fill-box;
    transform-origin: left center;
    animation: redacted 8s ease-out infinite;
  }

  @keyframes dot {
    0% {
      opacity: 0;
    }
    8% {
      opacity: 1;
    }
    92% {
      opacity: 0;
    }
  }
  @keyframes dot-1 {
    0% {
      opacity: 0;
    }
    12% {
      opacity: 1;
    }
    92% {
      opacity: 0;
    }
  }
  @keyframes dot-2 {
    0% {
      opacity: 0;
    }
    16% {
      opacity: 1;
    }
    92% {
      opacity: 0;
    }
  }
  @keyframes dot-3 {
    0% {
      opacity: 0;
    }
    20% {
      opacity: 1;
    }
    92% {
      opacity: 0;
    }
  }
  @keyframes dot-4 {
    0% {
      opacity: 0;
    }
    24% {
      opacity: 1;
    }
    92% {
      opacity: 0;
    }
  }
  @keyframes dot-5 {
    0% {
      opacity: 0;
    }
    28% {
      opacity: 1;
    }
    92% {
      opacity: 0;
    }
  }
  @keyframes dot-6 {
    0% {
      opacity: 0;
    }
    32% {
      opacity: 1;
    }
    92% {
      opacity: 0;
    }
  }
  @keyframes redacted {
    0%,
    40% {
      transform: scaleX(0);
    }
    52%,
    90% {
      transform: none;
      opacity: 1;
    }
    96%,
    100% {
      transform: none;
      opacity: 0;
    }
  }
</style>
