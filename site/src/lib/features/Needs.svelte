<script lang="ts">
  // An agent asking for you, drawn: your sessions, one under another; on the one your agent works in, a coral "needs
  // you" chip with its clock pops up. Your cursor comes and takes it, a check replaces the chip, and the agent's cursor
  // carries on to the next line. Decorative. It loops every 8 s, moving only transforms and opacity; for less motion it
  // stands still on the chip, you on it and the agent waiting beside it.
  import Cursor from './Cursor.svelte';
  import Window from './Window.svelte';
</script>

<Window>
  <!-- The sessions, each a row: its status dot and its name. -->
  {#each [34, 62, 90] as y (y)}
    <rect x="36" {y} width="248" height="22" rx="6" class="fill-wash" />
    <circle cx="48" cy={y + 11} r="3.5" class="fill-accent/40" />
    <rect x="58" y={y + 8} width={y === 62 ? 84 : 64} height="6" rx="3" class="fill-ink/15" />
  {/each}
  <rect x="36" y="62" width="248" height="22" rx="6" class="ring fill-none stroke-you" stroke-width="1.5" />

  <!-- Needs you: the chip, its clock and the time it asked. -->
  <g class="chip">
    <rect x="196" y="66" width="80" height="14" rx="7" class="fill-you" />
    <circle cx="206" cy="73" r="3.6" class="fill-none stroke-white" stroke-width="1.3" />
    <path d="M206 71v2.2l1.4 1" class="fill-none stroke-white" stroke-width="1.1" stroke-linecap="round" />
    <rect x="215" y="71" width="52" height="4" rx="2" class="fill-white/80" />
  </g>
  <g class="check">
    <circle cx="264" cy="73" r="7" class="fill-accent" />
    <path d="m260.5 73 2.5 2.5 4.5-5" class="fill-none stroke-white" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
  </g>

  <g class="agent"><Cursor /></g>
  <g class="you"><Cursor you /></g>
</Window>

<style>
  /* The still frame: the chip up, you on it, the agent waiting at its line. */
  .chip,
  .check {
    transform-box: fill-box;
    transform-origin: center;
  }
  .chip {
    animation: chip 8s ease-out infinite;
  }
  .ring {
    animation: ring 8s ease-out infinite;
  }
  .check {
    opacity: 0;
    animation: check 8s ease-out infinite;
  }
  .you {
    transform: translate(236px, 76px);
    animation: you 8s ease-in-out infinite;
  }
  .agent {
    transform: translate(150px, 76px);
    animation: agent 8s ease-in-out infinite;
  }

  @keyframes chip {
    0%,
    14% {
      transform: scale(0);
      opacity: 0;
    }
    19% {
      transform: scale(1.12);
      opacity: 1;
    }
    23%,
    56% {
      transform: scale(1);
      opacity: 1;
    }
    60%,
    100% {
      transform: scale(0.9);
      opacity: 0;
    }
  }
  @keyframes ring {
    0%,
    14% {
      opacity: 0;
    }
    19%,
    56% {
      opacity: 1;
    }
    62%,
    100% {
      opacity: 0;
    }
  }
  @keyframes check {
    0%,
    58% {
      transform: scale(0);
      opacity: 0;
    }
    62% {
      transform: scale(1.15);
      opacity: 1;
    }
    66%,
    92% {
      transform: scale(1);
      opacity: 1;
    }
    97%,
    100% {
      opacity: 0;
    }
  }
  @keyframes you {
    0%,
    28% {
      transform: translate(330px, 136px);
      opacity: 0;
    }
    31% {
      opacity: 1;
    }
    44% {
      transform: translate(236px, 76px) scale(1);
    }
    50% {
      transform: translate(236px, 76px) scale(0.85);
    }
    54%,
    60% {
      transform: translate(236px, 76px) scale(1);
      opacity: 1;
    }
    70%,
    100% {
      transform: translate(330px, 136px);
      opacity: 0;
    }
  }
  @keyframes agent {
    0% {
      transform: translate(80px, 50px);
      opacity: 0;
    }
    4% {
      opacity: 1;
    }
    13%,
    62% {
      transform: translate(150px, 76px);
    }
    19%,
    58% {
      opacity: 0.55;
    }
    64% {
      opacity: 1;
    }
    80%,
    90% {
      transform: translate(130px, 102px);
      opacity: 1;
    }
    97%,
    100% {
      transform: translate(130px, 102px);
      opacity: 0;
    }
  }
</style>
