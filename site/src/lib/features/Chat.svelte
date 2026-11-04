<script lang="ts">
  // A chat, drawn: a browser with its page, and a terminal rising over its lower half; a question typed in it, an answer,
  // and the coral cursor of the person at it. Decorative. It loops every 6 s, moving only transforms and opacity; for less
  // motion it stands still with the terminal up.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <defs>
    <clipPath id="chat-window"><rect x="56" y="6" width="208" height="116" rx="12" /></clipPath>
  </defs>
  <rect x="56" y="6" width="208" height="116" rx="12" class="fill-paper stroke-line" stroke-width="1.5" />
  <g clip-path="url(#chat-window)">
    <!-- The browser: its bar and its page. -->
    <rect x="56" y="6" width="208" height="18" class="fill-accent-soft" />
    <circle cx="68" cy="15" r="2.5" class="fill-accent/40" />
    <rect x="78" y="11.5" width="120" height="7" rx="3.5" class="fill-paper" />
    <rect x="72" y="33" width="84" height="7" rx="3.5" class="fill-ink/20" />
    <rect x="72" y="45" width="176" height="5" rx="2.5" class="fill-wash" />
    <rect x="72" y="54" width="150" height="5" rx="2.5" class="fill-wash" />

    <!-- The terminal over it. -->
    <g class="sheet">
      <path d="M56 74a10 10 0 0 1 10-10h188a10 10 0 0 1 10 10v48H56z" class="fill-wash stroke-line" stroke-width="1.5" />
      <rect x="68" y="72" width="46" height="5" rx="2.5" class="fill-ink/25" />
      <path d="m247 73 3.5 3.5 3.5-3.5" class="fill-none stroke-ink/40" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
      <path d="m68 87 3 2.5-3 2.5" class="fill-none stroke-accent" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
      <rect x="76" y="87" width="104" height="5" rx="2.5" class="asked fill-accent/70" />
      <rect x="68" y="98" width="150" height="5" rx="2.5" class="said fill-ink/20" />
      <rect x="68" y="107" width="96" height="5" rx="2.5" class="said fill-ink/20" />
      <rect x="170" y="106" width="4" height="7" rx="1" class="cursor fill-you" />
    </g>
  </g>
</svg>

<style>
  .sheet {
    animation: sheet 6s cubic-bezier(0.2, 0.7, 0.2, 1) infinite;
  }
  .asked {
    animation: appear 6s ease-out infinite;
    animation-delay: 0.4s;
  }
  .said {
    animation: appear 6s ease-out infinite;
    animation-delay: 1s;
  }
  .cursor {
    animation: appear 6s ease-out infinite;
    animation-delay: 1.4s;
  }

  @keyframes sheet {
    0%,
    6% {
      transform: translateY(58px);
    }
    18%,
    92% {
      transform: none;
    }
    100% {
      transform: translateY(58px);
    }
  }
  @keyframes appear {
    0%,
    14% {
      opacity: 0;
    }
    22%,
    84% {
      opacity: 1;
    }
    92%,
    100% {
      opacity: 0;
    }
  }
</style>
