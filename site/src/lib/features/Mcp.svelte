<script lang="ts">
  // The MCP card, drawn: your agent's terminal on the left, the browser on the right; a cable runs from the agent, its
  // plug slides into the browser's socket, a pulse runs along it and the page fills in. Decorative. It loops every
  // 7 s, moving only transforms and opacity; for less motion it stands still plugged in, the page filled.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The agent's terminal: dark, a prompt and two lines. -->
  <rect x="20" y="22" width="104" height="84" rx="10" class="fill-void" />
  <path d="m32 40 6 4-6 4" class="fill-none stroke-accent-soft" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
  <rect x="44" y="41" width="52" height="6" rx="3" class="fill-accent" />
  <rect x="32" y="58" width="72" height="5" rx="2.5" class="fill-white/25" />
  <rect x="32" y="70" width="56" height="5" rx="2.5" class="fill-white/25" />
  <rect x="32" y="82" width="64" height="5" rx="2.5" class="fill-white/25" />

  <!-- The browser, its socket on the left edge. -->
  <rect x="196" y="22" width="104" height="84" rx="10" class="fill-paper stroke-line" stroke-width="1.5" />
  <path d="M206 22h84a10 10 0 0 1 10 10v4H196v-4a10 10 0 0 1 10-10z" class="fill-accent-soft" />
  <g class="page">
    <rect x="210" y="48" width="56" height="7" rx="3.5" class="fill-ink/15" />
    <rect x="210" y="62" width="76" height="5" rx="2.5" class="fill-wash" />
    <rect x="210" y="72" width="64" height="5" rx="2.5" class="fill-wash" />
    <rect x="210" y="86" width="36" height="12" rx="6" class="fill-accent" />
  </g>
  <rect x="188" y="56" width="10" height="20" rx="3" class="fill-ink" />

  <!-- The cable, then the plug at its end. -->
  <path d="M124 66h28" class="fill-none stroke-ink" stroke-width="3" stroke-linecap="round" />
  <g class="plug">
    <path d="M124 66h40" class="fill-none stroke-ink" stroke-width="3" stroke-linecap="round" />
    <rect x="160" y="57" width="22" height="18" rx="4" class="fill-accent" />
    <path d="M182 61h6M182 71h6" class="stroke-ink" stroke-width="2.5" stroke-linecap="round" />
  </g>
  <circle cx="150" cy="66" r="4" class="spark fill-accent" />
</svg>

<style>
  .plug {
    animation: plug 7s cubic-bezier(0.5, 0, 0.25, 1) infinite;
  }
  .spark {
    opacity: 0;
    animation: spark 7s ease-in-out infinite;
  }
  .page {
    animation: page 7s ease-out infinite;
  }

  @keyframes plug {
    0%,
    10% {
      transform: translateX(-36px);
    }
    30%,
    88% {
      transform: none;
    }
    98%,
    100% {
      transform: translateX(-36px);
    }
  }
  @keyframes spark {
    0%,
    32% {
      transform: translateX(-24px);
      opacity: 0;
    }
    36% {
      opacity: 1;
    }
    46% {
      transform: translateX(44px);
      opacity: 0;
    }
    100% {
      opacity: 0;
    }
  }
  @keyframes page {
    0%,
    42% {
      transform: translateY(4px);
      opacity: 0;
    }
    54%,
    90% {
      transform: none;
      opacity: 1;
    }
    98%,
    100% {
      opacity: 0;
    }
  }
</style>
