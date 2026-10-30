<script lang="ts">
  // Two-step sign-in, drawn: a sign-in card takes the password (dots), then the six boxes of an authenticator's code
  // fill one by one, and the card opens with a check; beside it, a passkey (a fingerprint) glows as the other way in.
  // Decorative. It loops every 8 s, moving only transforms and opacity; for less motion it stands still signed in.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <rect x="40" y="14" width="160" height="100" rx="12" class="fill-paper stroke-line" stroke-width="1.5" />
  <!-- The password. -->
  <rect x="56" y="28" width="128" height="22" rx="6" class="fill-paper stroke-line" stroke-width="1.5" />
  {#each [0, 1, 2, 3, 4, 5, 6, 7] as i (i)}<circle cx={68 + i * 11} cy="39" r="2.6" class="fill-ink" />{/each}
  <!-- The code, digit by digit. -->
  {#each [0, 1, 2, 3, 4, 5] as i (i)}
    <rect x={56 + i * 22} y="60" width="18" height="24" rx="5" class="fill-paper stroke-line" stroke-width="1.5" />
    <rect x={61 + i * 22} y="69" width="8" height="6" rx="2" class="digit g{i} fill-accent" />
  {/each}
  <!-- Signed in. -->
  <g class="in">
    <rect x="56" y="92" width="128" height="14" rx="7" class="fill-accent" />
    <path d="m114 99 3 3 6-6" class="fill-none stroke-white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
  </g>

  <!-- A passkey. -->
  <g class="key fill-none stroke-accent" stroke-width="2.2" stroke-linecap="round">
    <path d="M248 46a18 18 0 0 1 22 6M240 58a18 18 0 0 1 4-8M272 60c0 14-3 22-6 28" />
    <path d="M246 88c3-7 4-15 4-24a8 8 0 0 1 16 0c0 4 0 8-1 12" />
    <path d="M258 64c0 12-2 20-6 28" />
  </g>
</svg>

<style>
  .digit {
    animation: digit 8s step-end infinite;
  }
  .g1 {
    animation-delay: 0.3s;
  }
  .g2 {
    animation-delay: 0.6s;
  }
  .g3 {
    animation-delay: 0.9s;
  }
  .g4 {
    animation-delay: 1.2s;
  }
  .g5 {
    animation-delay: 1.5s;
  }
  .in {
    animation: in 8s ease-out infinite;
  }
  .key {
    animation: key 8s ease-in-out infinite;
  }

  @keyframes digit {
    0% {
      opacity: 0;
    }
    20% {
      opacity: 1;
    }
    90% {
      opacity: 0;
    }
  }
  @keyframes in {
    0%,
    48% {
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
  @keyframes key {
    0%,
    56%,
    100% {
      opacity: 1;
    }
    70% {
      opacity: 0.35;
    }
  }
</style>
