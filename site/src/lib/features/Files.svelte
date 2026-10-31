<script lang="ts">
  // The agent's files, drawn: you allow it (your tick on its question), its own file comes into the browser, and its
  // pointer gives it to the page's upload button, whose file then shows on the page. Decorative. It loops every 8 s,
  // moving only transforms and opacity; for less motion it stands still with the file on the page.
  import Cursor from './Cursor.svelte';
  import Window from './Window.svelte';
</script>

<Window>
  <rect x="40" y="36" width="80" height="8" rx="4" class="fill-ink/15" />
  <rect x="40" y="56" width="70" height="20" rx="10" class="press fill-accent" />
  <path d="M75 70v-8m-3.5 3.5L75 62l3.5 3.5" class="fill-none stroke-white" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
  <g class="chosen">
    <path d="M120 58h9l4 4v12h-13z" class="fill-paper stroke-ink" stroke-width="1.5" stroke-linejoin="round" />
    <rect x="140" y="63" width="64" height="6" rx="3" class="fill-ink/25" />
  </g>

  <!-- The agent's question, and your answer. -->
  <g class="ask">
    <rect x="176" y="86" width="112" height="28" rx="14" class="fill-paper stroke-ink/30" stroke-width="1.5" />
    <rect x="188" y="97" width="52" height="6" rx="3" class="fill-ink/25" />
    <circle cx="270" cy="100" r="8" class="fill-you" />
    <path d="m266 100 3 3 5-6" class="fill-none stroke-white" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
  </g>

  <!-- Its file, on its way in. -->
  <g class="file">
    <path d="M250 40h12l6 6v16h-18z" class="fill-paper stroke-ink" stroke-width="1.6" stroke-linejoin="round" />
    <rect x="253" y="52" width="10" height="2.5" rx="1.25" class="fill-accent" />
  </g>

  <g class="agent"><Cursor /></g>
</Window>

<style>
  .agent {
    transform: translate(100px, 72px);
    animation: agent 8s ease-in-out infinite;
  }
  .press {
    transform-box: fill-box;
    transform-origin: center;
    animation: press 8s ease-in-out infinite;
  }
  .ask {
    opacity: 0;
    animation: ask 8s cubic-bezier(0.3, 0, 0.2, 1) infinite;
  }
  .file {
    opacity: 0;
    animation: file 8s ease-in-out infinite;
  }
  .chosen {
    animation: chosen 8s ease-out infinite;
  }

  @keyframes ask {
    0%,
    4% {
      transform: translateY(10px);
      opacity: 0;
    }
    10%,
    30% {
      transform: none;
      opacity: 1;
    }
    36%,
    100% {
      transform: translateY(10px);
      opacity: 0;
    }
  }
  @keyframes file {
    0%,
    30% {
      transform: translateX(40px);
      opacity: 0;
    }
    36%,
    46% {
      transform: none;
      opacity: 1;
    }
    60% {
      transform: translate(-150px, 18px) scale(0.7);
      opacity: 1;
    }
    64%,
    100% {
      transform: translate(-150px, 18px) scale(0.7);
      opacity: 0;
    }
  }
  @keyframes agent {
    0%,
    40% {
      transform: translate(170px, 110px);
      opacity: 0;
    }
    46% {
      opacity: 1;
    }
    58% {
      transform: translate(84px, 70px) scale(1);
    }
    61% {
      transform: translate(84px, 70px) scale(0.85);
    }
    64%,
    90% {
      transform: translate(100px, 72px) scale(1);
      opacity: 1;
    }
    98%,
    100% {
      transform: translate(100px, 72px);
      opacity: 0;
    }
  }
  @keyframes press {
    0%,
    59% {
      transform: none;
    }
    61% {
      transform: scale(0.92);
    }
    64%,
    100% {
      transform: none;
    }
  }
  @keyframes chosen {
    0%,
    64% {
      transform: translateX(-6px);
      opacity: 0;
    }
    70%,
    92% {
      transform: none;
      opacity: 1;
    }
    98%,
    100% {
      opacity: 0;
    }
  }
</style>
