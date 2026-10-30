<script lang="ts">
  // How the agent sees, drawn: a page whose parts the agent reads one by one (each outlined in blue, as its
  // accessibility tree names it), then its cursor goes to the button and clicks it. Decorative. It loops every 8 s,
  // moving only transforms and opacity; for less motion it stands still with every part outlined and the cursor on the
  // button.
  import Cursor from './Cursor.svelte';
  import Window from './Window.svelte';
</script>

<Window>
  <rect x="40" y="36" width="88" height="9" rx="4.5" class="fill-ink/15" />
  <rect x="40" y="54" width="132" height="16" rx="5" class="fill-paper stroke-line" stroke-width="1.5" />
  <rect x="40" y="78" width="132" height="16" rx="5" class="fill-paper stroke-line" stroke-width="1.5" />
  <rect x="200" y="54" width="72" height="40" rx="8" class="fill-wash" />
  <rect x="40" y="100" width="48" height="14" rx="7" class="button fill-accent" />

  <!-- What the agent reads: an outline around each part, in turn. -->
  <rect x="36" y="32" width="96" height="17" rx="5" class="seen a fill-none stroke-accent" stroke-width="1.5" stroke-dasharray="4 3" />
  <rect x="36" y="50" width="140" height="24" rx="7" class="seen b fill-none stroke-accent" stroke-width="1.5" stroke-dasharray="4 3" />
  <rect x="36" y="74" width="140" height="24" rx="7" class="seen c fill-none stroke-accent" stroke-width="1.5" stroke-dasharray="4 3" />
  <rect x="196" y="50" width="80" height="48" rx="10" class="seen d fill-none stroke-accent" stroke-width="1.5" stroke-dasharray="4 3" />
  <rect x="36" y="97" width="56" height="20" rx="10" class="seen e fill-none stroke-accent" stroke-width="1.5" stroke-dasharray="4 3" />

  <g class="agent"><Cursor /></g>
</Window>

<style>
  .seen {
    animation: seen 8s ease-out infinite;
  }
  .b {
    animation-name: seen-b;
  }
  .c {
    animation-name: seen-c;
  }
  .d {
    animation-name: seen-d;
  }
  .e {
    animation-name: seen-e;
  }
  .agent {
    transform: translate(70px, 108px);
    animation: agent 8s ease-in-out infinite;
  }
  .button {
    transform-box: fill-box;
    transform-origin: center;
    animation: press 8s ease-in-out infinite;
  }

  @keyframes seen {
    0%,
    6% {
      opacity: 0;
    }
    10%,
    88% {
      opacity: 1;
    }
    96%,
    100% {
      opacity: 0;
    }
  }
  @keyframes seen-b {
    0%,
    14% {
      opacity: 0;
    }
    18%,
    88% {
      opacity: 1;
    }
    96%,
    100% {
      opacity: 0;
    }
  }
  @keyframes seen-c {
    0%,
    22% {
      opacity: 0;
    }
    26%,
    88% {
      opacity: 1;
    }
    96%,
    100% {
      opacity: 0;
    }
  }
  @keyframes seen-d {
    0%,
    30% {
      opacity: 0;
    }
    34%,
    88% {
      opacity: 1;
    }
    96%,
    100% {
      opacity: 0;
    }
  }
  @keyframes seen-e {
    0%,
    38% {
      opacity: 0;
    }
    42%,
    88% {
      opacity: 1;
    }
    96%,
    100% {
      opacity: 0;
    }
  }
  @keyframes agent {
    0%,
    44% {
      transform: translate(250px, 112px);
      opacity: 0;
    }
    50% {
      opacity: 1;
    }
    64% {
      transform: translate(70px, 108px) scale(1);
    }
    68% {
      transform: translate(70px, 108px) scale(0.84);
    }
    72%,
    90% {
      transform: translate(70px, 108px) scale(1);
      opacity: 1;
    }
    96%,
    100% {
      transform: translate(70px, 108px);
      opacity: 0;
    }
  }
  @keyframes press {
    0%,
    66% {
      transform: none;
    }
    68% {
      transform: scale(0.9);
    }
    72%,
    100% {
      transform: none;
    }
  }
</style>
