<script lang="ts">
  // One port, drawn: your browser on the left, my server's one HTTPS port on the right, and two lanes between them. The
  // media run fast on the UDP lane; a block falls on it and they move to the other lane on their own, then go back when
  // it lifts. Decorative. It loops every 9 s, moving only transforms and opacity; for less motion it stands still on the
  // UDP lane.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- Your browser. -->
  <rect x="16" y="34" width="64" height="60" rx="8" class="fill-paper stroke-line" stroke-width="1.5" />
  <path d="M24 34h48a8 8 0 0 1 8 8v4H16v-4a8 8 0 0 1 8-8z" class="fill-accent-soft" />
  <!-- My server and its one port. -->
  <rect x="240" y="30" width="64" height="68" rx="8" class="fill-ink" />
  <path d="M252 46h40M252 58h40" class="stroke-white/25" stroke-width="3" stroke-linecap="round" />
  <rect x="234" y="72" width="16" height="16" rx="4" class="fill-accent" />

  <!-- The UDP lane, and the WebSocket lane under it. -->
  <path d="M84 52h150" class="fill-none stroke-accent/30" stroke-width="2" stroke-dasharray="3 5" />
  <path d="M84 80h150" class="fill-none stroke-line" stroke-width="2" />
  <g class="fast">{#each [0, 1, 2] as i (i)}<circle cx={96 + i * 46} cy="52" r="3.5" class="dot fill-accent" />{/each}</g>
  <g class="slow">{#each [0, 1, 2] as i (i)}<circle cx={96 + i * 46} cy="80" r="3.5" class="dot fill-accent" />{/each}</g>
  <rect x="150" y="40" width="10" height="24" rx="3" class="block fill-you" />
</svg>

<style>
  .dot {
    animation: run 1.5s linear infinite;
  }
  .fast {
    animation: fast 9s step-end infinite;
  }
  .slow {
    opacity: 0;
    animation: slow 9s step-end infinite;
  }
  .block {
    opacity: 0;
    animation: block 9s ease-in-out infinite;
  }

  @keyframes run {
    from {
      transform: none;
    }
    to {
      transform: translateX(46px);
    }
  }
  @keyframes fast {
    0% {
      opacity: 1;
    }
    38% {
      opacity: 0;
    }
    76% {
      opacity: 1;
    }
  }
  @keyframes slow {
    0% {
      opacity: 0;
    }
    38% {
      opacity: 1;
    }
    76% {
      opacity: 0;
    }
  }
  @keyframes block {
    0%,
    30% {
      transform: translateY(-18px);
      opacity: 0;
    }
    36%,
    70% {
      transform: none;
      opacity: 1;
    }
    76%,
    100% {
      transform: translateY(-18px);
      opacity: 0;
    }
  }
</style>
