<script lang="ts">
  // A notification, drawn: your team's chat, two quiet messages in it; a new one slides in from easi.run, a coral line
  // (an agent needs you) and a blue link to the session, with the seal of its signature; the bell above rings.
  // Decorative. It loops every 6 s, moving only transforms and opacity; for less motion it stands still with the
  // message in.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The chat: its header with the channel and the bell, then the messages. -->
  <rect x="56" y="6" width="208" height="116" rx="12" class="fill-paper stroke-line" stroke-width="1.5" />
  <path d="M68 6h184a12 12 0 0 1 12 12v6H56v-6A12 12 0 0 1 68 6z" class="fill-accent-soft" />
  <path d="M70 12.5l-1.6 11M75 12.5l-1.6 11M67.5 16h9M66.8 20.5h9" class="fill-none stroke-accent" stroke-width="1.3" stroke-linecap="round" />
  <rect x="84" y="13" width="56" height="6" rx="3" class="fill-paper" />
  <g class="bell">
    <path d="M244 21.5h-12c1.6-1.6 2-3 2-5.5a4 4 0 0 1 8 0c0 2.5.4 3.9 2 5.5z" class="fill-accent" />
    <circle cx="238" cy="23" r="1.6" class="fill-accent" />
  </g>

  {#each [32, 54] as y (y)}
    <circle cx="74" cy={y + 6} r="6" class="fill-wash" />
    <rect x="86" y={y} width="56" height="5" rx="2.5" class="fill-ink/15" />
    <rect x="86" y={y + 9} width={y === 32 ? 120 : 96} height="5" rx="2.5" class="fill-wash" />
  {/each}

  <!-- The new message: from easi.run, what is needed, the link, and the signature's seal. -->
  <g class="message">
    <rect x="64" y="74" width="192" height="40" rx="8" class="fill-wash" />
    <circle cx="78" cy="88" r="7" class="fill-accent" />
    <path d="M75 85.2 81.6 88.6 78.6 89.5 77.3 92.6Z" class="fill-white" />
    <rect x="92" y="82" width="60" height="5" rx="2.5" class="fill-ink/25" />
    <rect x="92" y="92" width="104" height="5" rx="2.5" class="fill-you" />
    <rect x="92" y="102" width="72" height="5" rx="2.5" class="fill-accent/60" />
    <circle cx="236" cy="94" r="8" class="fill-accent-soft stroke-accent" stroke-width="1.3" />
    <path d="m232.6 94 2.4 2.4 4.2-4.8" class="fill-none stroke-accent" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
  </g>
</svg>

<style>
  .message {
    animation: message 6s cubic-bezier(0.3, 0, 0.2, 1) infinite;
  }
  .bell {
    transform-box: fill-box;
    transform-origin: 50% 0;
    animation: bell 6s ease-in-out infinite;
  }

  @keyframes message {
    0%,
    12% {
      transform: translateY(14px);
      opacity: 0;
    }
    22%,
    90% {
      transform: none;
      opacity: 1;
    }
    97%,
    100% {
      opacity: 0;
    }
  }
  @keyframes bell {
    0%,
    20% {
      transform: none;
    }
    24% {
      transform: rotate(16deg);
    }
    28% {
      transform: rotate(-14deg);
    }
    32% {
      transform: rotate(10deg);
    }
    36% {
      transform: rotate(-6deg);
    }
    40%,
    100% {
      transform: none;
    }
  }
</style>
