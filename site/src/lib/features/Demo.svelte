<script lang="ts">
  // Trying it now, drawn: a window with a play button, which your cursor presses; the page comes alive, an agent's
  // cursor at work in it. Decorative. It loops every 8 s, moving only transforms and opacity; for less motion it stands
  // still on the play button.
  import Cursor from './Cursor.svelte';
  import Window from './Window.svelte';
</script>

<Window>
  <!-- The live page. -->
  <g class="page">
    <rect x="40" y="38" width="80" height="8" rx="4" class="fill-ink/15" />
    <rect x="40" y="54" width="132" height="5" rx="2.5" class="fill-wash" />
    <rect x="40" y="64" width="116" height="5" rx="2.5" class="fill-wash" />
    <rect x="40" y="84" width="44" height="14" rx="7" class="fill-accent" />
    <rect x="192" y="38" width="88" height="62" rx="8" class="fill-wash" />
    <g class="agent"><Cursor /></g>
  </g>
  <!-- The play button. -->
  <g class="play">
    <circle cx="160" cy="70" r="24" class="fill-accent" />
    <circle cx="160" cy="70" r="24" class="ring fill-none stroke-accent" stroke-width="2" />
    <path d="M153 60v20l17-10z" class="fill-white" />
  </g>
  <g class="you"><Cursor you /></g>
</Window>

<style>
  .page {
    opacity: 0;
    animation: page 8s ease-out infinite;
  }
  .play {
    transform-box: fill-box;
    transform-origin: center;
    animation: play 8s ease-in-out infinite;
  }
  .ring {
    transform-box: fill-box;
    transform-origin: center;
    animation: ring 2s ease-out infinite;
  }
  .you {
    transform: translate(166px, 76px);
    opacity: 0;
    animation: you 8s ease-in-out infinite;
  }
  .agent {
    transform: translate(70px, 92px);
    animation: agent 8s ease-in-out infinite;
  }

  @keyframes play {
    0%,
    24% {
      transform: none;
      opacity: 1;
    }
    27% {
      transform: scale(0.9);
    }
    34%,
    92% {
      transform: scale(1.3);
      opacity: 0;
    }
    100% {
      transform: none;
      opacity: 1;
    }
  }
  @keyframes ring {
    from {
      transform: none;
      opacity: 0.7;
    }
    to {
      transform: scale(1.6);
      opacity: 0;
    }
  }
  @keyframes you {
    0% {
      transform: translate(280px, 116px);
      opacity: 0;
    }
    6% {
      opacity: 1;
    }
    22% {
      transform: translate(166px, 76px) scale(1);
    }
    25% {
      transform: translate(166px, 76px) scale(0.85);
    }
    28% {
      transform: translate(166px, 76px) scale(1);
      opacity: 1;
    }
    36%,
    100% {
      transform: translate(166px, 76px);
      opacity: 0;
    }
  }
  @keyframes page {
    0%,
    30% {
      opacity: 0;
    }
    38%,
    92% {
      opacity: 1;
    }
    100% {
      opacity: 0;
    }
  }
  @keyframes agent {
    0%,
    40% {
      transform: translate(230px, 70px);
    }
    64% {
      transform: translate(62px, 92px);
    }
    84%,
    100% {
      transform: translate(230px, 70px);
    }
  }
</style>
