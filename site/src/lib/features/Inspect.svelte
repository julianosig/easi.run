<script lang="ts">
  // How the agent reads a page's structure, drawn: the part a selector matches is outlined, and its tag comes back beside
  // the page, with nothing on the page moving. Decorative. It loops every 8 s, moving only opacity; for less motion it
  // stands still with the part outlined and its answer shown.
  import Window from './Window.svelte';
</script>

<Window>
  <rect x="40" y="36" width="88" height="9" rx="4.5" class="fill-ink/15" />
  <rect x="40" y="54" width="120" height="16" rx="5" class="fill-paper stroke-line" stroke-width="1.5" />
  <rect x="40" y="78" width="120" height="16" rx="5" class="fill-paper stroke-line" stroke-width="1.5" />
  <rect x="40" y="100" width="48" height="14" rx="7" class="fill-accent" />

  <rect x="36" y="74" width="128" height="24" rx="7" class="found fill-none stroke-accent" stroke-width="1.5" stroke-dasharray="4 3" />

  <g class="answer">
    <rect x="180" y="50" width="96" height="66" rx="8" class="fill-wash" />
    <rect x="190" y="60" width="46" height="6" rx="3" class="fill-accent" />
    <rect x="198" y="72" width="66" height="6" rx="3" class="fill-ink/25" />
    <rect x="198" y="84" width="52" height="6" rx="3" class="fill-ink/25" />
    <rect x="190" y="96" width="34" height="6" rx="3" class="fill-accent" />
  </g>
</Window>

<style>
  .found {
    animation: found 8s ease-out infinite;
  }
  .answer {
    animation: answer 8s ease-out infinite;
  }
  @keyframes found {
    0%,
    12% {
      opacity: 0;
    }
    22%,
    88% {
      opacity: 1;
    }
    100% {
      opacity: 0;
    }
  }
  @keyframes answer {
    0%,
    28% {
      opacity: 0;
    }
    40%,
    88% {
      opacity: 1;
    }
    100% {
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .found,
    .answer {
      animation: none;
    }
  }
</style>
