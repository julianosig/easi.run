<script lang="ts">
  // Off the open internet, drawn: my server with its lock; requests from the open internet (the globe) bounce off it,
  // while your own way in, a tunnel from your laptop, carries you straight through. Decorative. It loops every 8 s,
  // moving only transforms and opacity; for less motion it stands still with the tunnel open.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The open internet. -->
  <g transform="translate(44 40)" class="fill-none stroke-muted/70" stroke-width="2.2">
    <circle r="20" />
    <path d="M-20 0h40M0 -20c6 5 9 12 9 20s-3 15-9 20c-6-5-9-12-9-20s3-15 9-20z" />
  </g>
  <circle cx="70" cy="44" r="3.5" class="knock fill-muted" />

  <!-- My server, locked. -->
  <rect x="124" y="20" width="72" height="88" rx="10" class="fill-ink" />
  <path d="M136 38h48M136 50h48" class="stroke-white/25" stroke-width="3" stroke-linecap="round" />
  <g transform="translate(160 76)">
    <path d="M-6 -2v-4a6 6 0 0 1 12 0v4" class="fill-none stroke-white" stroke-width="2.4" />
    <rect x="-9" y="-2" width="18" height="14" rx="3" class="fill-white" />
  </g>

  <!-- Your laptop, and its tunnel in. -->
  <rect x="244" y="62" width="56" height="36" rx="4" class="fill-paper stroke-ink" stroke-width="2" />
  <path d="M236 102h72l-5 6h-62z" class="fill-ink" />
  <path d="M240 80h-44" class="fill-none stroke-you/40" stroke-width="10" stroke-linecap="round" />
  <circle cx="236" cy="80" r="3.5" class="through fill-you" />
</svg>

<style>
  .knock {
    opacity: 0;
    animation: knock 8s ease-in-out infinite;
  }
  .through {
    opacity: 0;
    animation: through 8s ease-in-out infinite;
  }

  @keyframes knock {
    0%,
    6% {
      transform: none;
      opacity: 0;
    }
    10% {
      opacity: 1;
    }
    24% {
      transform: translateX(48px);
      opacity: 1;
    }
    36% {
      transform: translate(20px, -8px);
      opacity: 0;
    }
    100% {
      opacity: 0;
    }
  }
  @keyframes through {
    0%,
    44% {
      transform: none;
      opacity: 0;
    }
    48% {
      opacity: 1;
    }
    66% {
      transform: translateX(-40px);
      opacity: 1;
    }
    72%,
    100% {
      transform: translateX(-40px);
      opacity: 0;
    }
  }
</style>
