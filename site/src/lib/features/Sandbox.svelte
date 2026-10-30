<script lang="ts">
  // The sandbox, drawn: Chrome inside its container, inside a second wall; a packet heads out on its own and stops at
  // the wall, while the one road out (the proxy's) stays lit; the shield on the wall pulses. Decorative. It loops every
  // 7 s, moving only transforms and opacity; for less motion it stands still, walled.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The walls. -->
  <rect x="92" y="10" width="136" height="108" rx="16" class="fill-wash stroke-accent" stroke-width="2" stroke-dasharray="7 5" />
  <rect x="112" y="28" width="96" height="72" rx="12" class="fill-paper stroke-ink" stroke-width="2.5" />
  <!-- Chrome, as a plain ring. -->
  <circle cx="160" cy="64" r="20" class="fill-none stroke-accent" stroke-width="7" />
  <circle cx="160" cy="64" r="8" class="fill-accent" />
  <!-- A packet that tries to leave, and stops. -->
  <circle cx="112" cy="64" r="4" class="packet fill-you" />
  <path d="M86 58l8 12M94 58l-8 12" class="stop fill-none stroke-you" stroke-width="2.5" stroke-linecap="round" />
  <!-- The shield on the wall. -->
  <g class="shield">
    <path d="M228 50l14 5v10c0 9-6 15-14 18-8-3-14-9-14-18V55z" class="fill-accent" />
    <path d="m222 66 4.5 4.5 8-8" class="fill-none stroke-white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
  </g>
</svg>

<style>
  .packet {
    opacity: 0;
    animation: packet 7s ease-in infinite;
  }
  .stop {
    opacity: 0;
    animation: stop 7s ease-out infinite;
  }
  .shield {
    transform-box: fill-box;
    transform-origin: center;
    animation: shield 7s ease-in-out infinite;
  }

  @keyframes packet {
    0%,
    10% {
      transform: none;
      opacity: 0;
    }
    14% {
      opacity: 1;
    }
    34% {
      transform: translateX(-16px);
      opacity: 1;
    }
    40%,
    100% {
      transform: translateX(-16px);
      opacity: 0;
    }
  }
  @keyframes stop {
    0%,
    32% {
      opacity: 0;
    }
    36%,
    56% {
      opacity: 1;
    }
    64%,
    100% {
      opacity: 0;
    }
  }
  @keyframes shield {
    0%,
    32%,
    48%,
    100% {
      transform: none;
    }
    38% {
      transform: scale(1.12);
    }
  }
</style>
