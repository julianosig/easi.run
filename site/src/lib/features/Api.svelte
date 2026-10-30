<script lang="ts">
  // The REST API card, drawn: an agent's token on the left sends its calls along to the right, where its sessions open
  // one after the other; then the token is revoked (struck through) and its sessions close at once. Decorative. It
  // loops every 8 s, moving only transforms and opacity; for less motion it stands still with the token live and both
  // sessions open.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The token: a key and its hidden characters. -->
  <rect x="20" y="44" width="104" height="40" rx="10" class="fill-paper stroke-line" stroke-width="1.5" />
  <circle cx="40" cy="64" r="6" class="fill-none stroke-accent" stroke-width="2.4" />
  <path d="M46 64h12m-3 0v5" class="fill-none stroke-accent" stroke-width="2.4" stroke-linecap="round" />
  {#each [0, 1, 2, 3, 4] as i (i)}<circle cx={70 + i * 10} cy="64" r="2.6" class="fill-ink" />{/each}
  <path d="M28 64h88" class="strike fill-none stroke-you" stroke-width="3" stroke-linecap="round" />

  <!-- The calls on their way. -->
  <circle cx="136" cy="64" r="3.5" class="call one fill-accent" />
  <circle cx="136" cy="64" r="3.5" class="call two fill-accent" />

  <!-- Its sessions. -->
  <g class="session first">
    <rect x="190" y="18" width="110" height="44" rx="8" class="fill-paper stroke-line" stroke-width="1.5" />
    <path d="M198 18h94a8 8 0 0 1 8 8v2H190v-2a8 8 0 0 1 8-8z" class="fill-accent-soft" />
    <rect x="200" y="38" width="50" height="5" rx="2.5" class="fill-ink/15" />
    <rect x="200" y="48" width="80" height="4" rx="2" class="fill-wash" />
  </g>
  <g class="session second">
    <rect x="190" y="68" width="110" height="44" rx="8" class="fill-paper stroke-line" stroke-width="1.5" />
    <path d="M198 68h94a8 8 0 0 1 8 8v2H190v-2a8 8 0 0 1 8-8z" class="fill-accent-soft" />
    <rect x="200" y="88" width="64" height="5" rx="2.5" class="fill-ink/15" />
    <rect x="200" y="98" width="72" height="4" rx="2" class="fill-wash" />
  </g>
</svg>

<style>
  .strike {
    transform-box: fill-box;
    transform-origin: left center;
    transform: scaleX(0);
    animation: strike 8s ease-in-out infinite;
  }
  .call {
    opacity: 0;
    animation: call 8s ease-in-out infinite;
  }
  .two {
    animation-delay: 1.2s;
  }
  .session {
    animation: session 8s ease-out infinite;
  }
  .second {
    animation-name: second;
  }

  @keyframes strike {
    0%,
    66% {
      transform: scaleX(0);
    }
    72%,
    92% {
      transform: scaleX(1);
    }
    100% {
      transform: scaleX(0);
    }
  }
  @keyframes call {
    0%,
    4% {
      transform: none;
      opacity: 0;
    }
    8% {
      opacity: 1;
    }
    20% {
      transform: translateX(44px);
      opacity: 0;
    }
    100% {
      opacity: 0;
    }
  }
  @keyframes session {
    0%,
    18% {
      transform: translateX(8px);
      opacity: 0;
    }
    24%,
    72% {
      transform: none;
      opacity: 1;
    }
    78%,
    100% {
      transform: none;
      opacity: 0;
    }
  }
  @keyframes second {
    0%,
    34% {
      transform: translateX(8px);
      opacity: 0;
    }
    40%,
    72% {
      transform: none;
      opacity: 1;
    }
    78%,
    100% {
      transform: none;
      opacity: 0;
    }
  }
</style>
