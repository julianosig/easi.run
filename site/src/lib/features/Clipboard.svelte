<script lang="ts">
  // Copy and paste both ways, drawn: the page on the left, your computer on the right; a copied line travels from the
  // page to your computer, then another from your computer back to the page, each landing where it is pasted.
  // Decorative. It loops every 8 s, moving only transforms and opacity; for less motion it stands still with both lines
  // in place.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The page. -->
  <rect x="20" y="18" width="124" height="92" rx="9" class="fill-paper stroke-line" stroke-width="1.5" />
  <path d="M29 18h106a9 9 0 0 1 9 9v5H20v-5a9 9 0 0 1 9-9z" class="fill-accent-soft" />
  <rect x="32" y="46" width="72" height="8" rx="4" class="fill-accent/30" />
  <rect x="32" y="64" width="96" height="5" rx="2.5" class="fill-wash" />
  <rect x="32" y="84" width="100" height="14" rx="5" class="fill-paper stroke-line" stroke-width="1.5" />
  <rect x="38" y="88.5" width="56" height="5" rx="2.5" class="pasted-in fill-you/60" />

  <!-- Your computer. -->
  <rect x="184" y="22" width="116" height="74" rx="7" class="fill-paper stroke-ink" stroke-width="2" />
  <path d="M172 100h140l-8 10H180z" class="fill-ink" />
  <rect x="196" y="38" width="60" height="5" rx="2.5" class="fill-wash" />
  <rect x="196" y="50" width="92" height="12" rx="4" class="fill-paper stroke-line" stroke-width="1.5" />
  <rect x="200" y="53.5" width="72" height="5" rx="2.5" class="pasted-out fill-accent/30" />
  <rect x="196" y="72" width="64" height="8" rx="4" class="fill-you/30" />

  <!-- What travels: out to your computer, then back in. -->
  <rect x="32" y="46" width="72" height="8" rx="4" class="out fill-accent" />
  <rect x="196" y="72" width="64" height="8" rx="4" class="in fill-you" />
</svg>

<style>
  .out {
    opacity: 0;
    animation: out 8s ease-in-out infinite;
  }
  .in {
    opacity: 0;
    animation: in 8s ease-in-out infinite;
  }
  .pasted-out {
    animation: pasted-out 8s ease-out infinite;
  }
  .pasted-in {
    animation: pasted-in 8s ease-out infinite;
  }

  @keyframes out {
    0%,
    6% {
      transform: none;
      opacity: 0;
    }
    10% {
      opacity: 1;
    }
    34% {
      transform: translate(168px, 7.5px) scaleX(1);
      opacity: 1;
    }
    40%,
    100% {
      transform: translate(168px, 7.5px);
      opacity: 0;
    }
  }
  @keyframes in {
    0%,
    48% {
      transform: none;
      opacity: 0;
    }
    52% {
      opacity: 1;
    }
    76% {
      transform: translate(-158px, 16.5px);
      opacity: 1;
    }
    82%,
    100% {
      transform: translate(-158px, 16.5px);
      opacity: 0;
    }
  }
  @keyframes pasted-out {
    0%,
    34% {
      opacity: 0;
    }
    38%,
    92% {
      opacity: 1;
    }
    98%,
    100% {
      opacity: 0;
    }
  }
  @keyframes pasted-in {
    0%,
    76% {
      opacity: 0;
    }
    80%,
    92% {
      opacity: 1;
    }
    98%,
    100% {
      opacity: 0;
    }
  }
</style>
