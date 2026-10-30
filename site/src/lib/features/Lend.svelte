<script lang="ts">
  // Lending a login, drawn: a saved session (its check and key) between two agents. The one you allowed comes to it, a
  // check appears and it opens logged in; the other comes too, finds only an empty outline, and turns back. Decorative.
  // It loops every 8 s, moving only transforms and opacity; for less motion it stands still with the allowed agent at
  // the session.
  import Cursor from './Cursor.svelte';
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The saved session, as the vault keeps it. -->
  <g class="saved">
    <rect x="116" y="36" width="88" height="56" rx="10" class="fill-paper stroke-line" stroke-width="1.5" />
    <circle cx="140" cy="64" r="10" class="fill-accent" />
    <path d="m135.5 64 3.2 3.2 5.5-6.4" class="fill-none stroke-white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
    <circle cx="166" cy="64" r="5.5" class="fill-none stroke-ink" stroke-width="2.4" />
    <path d="M171.5 64h17m-4 0v5m4-5v4" class="fill-none stroke-ink" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" />
  </g>
  <!-- What the other agent finds: nothing but an outline. -->
  <rect x="116" y="36" width="88" height="56" rx="10" class="nothing fill-none stroke-muted/50" stroke-width="1.5" stroke-dasharray="5 4" />

  <!-- The agent you allowed, with its badge. -->
  <g class="allowed"><Cursor /></g>
  <g class="badge">
    <circle cx="112" cy="34" r="8" class="fill-you" />
    <path d="m108.5 34 2.5 2.5 4.5-5" class="fill-none stroke-white" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
  </g>
  <!-- The other, which turns back. -->
  <g class="other"><Cursor /></g>
</svg>

<style>
  .allowed {
    transform: translate(124px, 78px);
    animation: allowed 8s ease-in-out infinite;
  }
  .badge {
    transform-box: fill-box;
    transform-origin: center;
    animation: badge 8s ease-out infinite;
  }
  .other {
    opacity: 0;
    animation: other 8s ease-in-out infinite;
  }
  .saved {
    animation: saved 8s ease-in-out infinite;
  }
  .nothing {
    opacity: 0;
    animation: nothing 8s ease-in-out infinite;
  }

  @keyframes allowed {
    0% {
      transform: translate(30px, 100px);
      opacity: 0;
    }
    6% {
      opacity: 1;
    }
    24%,
    46% {
      transform: translate(124px, 78px);
      opacity: 1;
    }
    54%,
    100% {
      transform: translate(30px, 100px);
      opacity: 0;
    }
  }
  @keyframes badge {
    0%,
    24% {
      transform: scale(0);
    }
    29% {
      transform: scale(1.2);
    }
    32%,
    46% {
      transform: scale(1);
      opacity: 1;
    }
    52%,
    100% {
      transform: scale(1);
      opacity: 0;
    }
  }
  @keyframes saved {
    0%,
    54% {
      opacity: 1;
    }
    60%,
    88% {
      opacity: 0;
    }
    94%,
    100% {
      opacity: 1;
    }
  }
  @keyframes nothing {
    0%,
    56% {
      opacity: 0;
    }
    62%,
    86% {
      opacity: 1;
    }
    92%,
    100% {
      opacity: 0;
    }
  }
  @keyframes other {
    0%,
    54% {
      transform: translate(300px, 30px);
      opacity: 0;
    }
    58% {
      opacity: 1;
    }
    70%,
    76% {
      transform: translate(212px, 52px);
      opacity: 1;
    }
    90%,
    100% {
      transform: translate(300px, 30px);
      opacity: 0;
    }
  }
</style>
