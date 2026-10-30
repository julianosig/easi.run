<script lang="ts">
  // The watch link, drawn: a link chip arrives from your agent; your cursor clicks it, and it opens into the browser
  // live, its dot pulsing, the agent's cursor at work on the page. Decorative. It loops every 8 s, moving only
  // transforms and opacity; for less motion it stands still on the live browser.
  import Cursor from './Cursor.svelte';
  import Window from './Window.svelte';
</script>

<Window>
  <rect x="40" y="38" width="80" height="8" rx="4" class="fill-ink/15" />
  <rect x="40" y="54" width="120" height="5" rx="2.5" class="fill-wash" />
  <rect x="40" y="65" width="104" height="5" rx="2.5" class="fill-wash" />
  <rect x="40" y="76" width="112" height="5" rx="2.5" class="fill-wash" />
  <rect x="184" y="38" width="96" height="60" rx="8" class="fill-wash" />
  <!-- Live: a dot that pulses, on the toolbar. -->
  <circle cx="276" cy="15" r="3.5" class="fill-you" />
  <circle cx="276" cy="15" r="3.5" class="live fill-none stroke-you" stroke-width="1.5" />
  <g class="agent"><Cursor /></g>

  <!-- The link, laid over the window until it is opened. -->
  <g class="link">
    <rect x="24" y="6" width="272" height="116" rx="10" class="fill-wash" />
    <rect x="92" y="50" width="136" height="28" rx="14" class="fill-paper stroke-accent" stroke-width="1.5" />
    <path d="M110 64h4m0 0a4 4 0 0 1 0-8h4m-4 8h8m0 0a4 4 0 0 0 0-8h-4" class="fill-none stroke-accent" stroke-width="2" stroke-linecap="round" />
    <rect x="132" y="61" width="80" height="6" rx="3" class="fill-accent/40" />
    <g class="you"><Cursor you /></g>
  </g>
</Window>

<style>
  .live {
    transform-box: fill-box;
    transform-origin: center;
    animation: live 2s ease-out infinite;
  }
  .link {
    opacity: 0;
    animation: link 8s ease-in-out infinite;
  }
  .you {
    transform: translate(170px, 70px);
    animation: you 8s ease-in-out infinite;
  }
  .agent {
    transform: translate(232px, 72px);
    animation: agent 8s ease-in-out infinite;
  }

  @keyframes live {
    0% {
      transform: scale(1);
      opacity: 0.8;
    }
    100% {
      transform: scale(2.6);
      opacity: 0;
    }
  }
  @keyframes link {
    0%,
    30% {
      transform: none;
      opacity: 1;
    }
    40%,
    92% {
      transform: scale(1.04);
      opacity: 0;
    }
    100% {
      opacity: 1;
    }
  }
  @keyframes you {
    0% {
      transform: translate(260px, 116px);
    }
    20% {
      transform: translate(170px, 70px) scale(1);
    }
    24% {
      transform: translate(170px, 70px) scale(0.85);
    }
    28%,
    100% {
      transform: translate(170px, 70px) scale(1);
    }
  }
  @keyframes agent {
    0%,
    40% {
      transform: translate(200px, 52px);
    }
    60% {
      transform: translate(250px, 84px);
    }
    80%,
    100% {
      transform: translate(200px, 52px);
    }
  }
</style>
