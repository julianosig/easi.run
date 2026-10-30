<script lang="ts">
  // Making it yours, drawn: the session's screen in the middle, and your own interface assembling around it, your top
  // bar sliding in, then your side panel, then your logo appearing on the screen in place of mine. Decorative. It loops
  // every 8 s, moving only transforms and opacity; for less motion it stands still with your interface in place.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The session's screen, drawn by the viewer. -->
  <rect x="92" y="34" width="204" height="86" rx="6" class="fill-paper stroke-line" stroke-width="1.5" />
  <rect x="104" y="46" width="72" height="6" rx="3" class="fill-ink/15" />
  <rect x="104" y="58" width="112" height="4" rx="2" class="fill-wash" />
  <!-- Your logo, on its screen. -->
  <g class="logo">
    <rect x="222" y="76" width="56" height="30" rx="6" class="fill-you/15" />
    <path d="M238 98l8-14 8 14z" class="fill-you" />
    <circle cx="261" cy="91" r="6" class="fill-you/70" />
  </g>

  <!-- Your top bar. -->
  <g class="top">
    <rect x="24" y="8" width="272" height="20" rx="6" class="fill-void" />
    <circle cx="36" cy="18" r="4" class="fill-you" />
    <rect x="46" y="15" width="40" height="6" rx="3" class="fill-white/60" />
    <rect x="236" y="13" width="52" height="10" rx="5" class="fill-you" />
  </g>
  <!-- Your side panel. -->
  <g class="side">
    <rect x="24" y="34" width="62" height="86" rx="6" class="fill-ink/85" />
    {#each [46, 60, 74, 88] as y (y)}<rect x="34" y={y} width="42" height="6" rx="3" class="fill-white/35" />{/each}
  </g>
</svg>

<style>
  .top {
    animation: top 8s cubic-bezier(0.3, 0, 0.2, 1) infinite;
  }
  .side {
    animation: side 8s cubic-bezier(0.3, 0, 0.2, 1) infinite;
  }
  .logo {
    transform-box: fill-box;
    transform-origin: center;
    animation: logo 8s ease-out infinite;
  }

  @keyframes top {
    0%,
    6% {
      transform: translateY(-14px);
      opacity: 0;
    }
    18%,
    90% {
      transform: none;
      opacity: 1;
    }
    98%,
    100% {
      opacity: 0;
    }
  }
  @keyframes side {
    0%,
    20% {
      transform: translateX(-14px);
      opacity: 0;
    }
    32%,
    90% {
      transform: none;
      opacity: 1;
    }
    98%,
    100% {
      opacity: 0;
    }
  }
  @keyframes logo {
    0%,
    36% {
      transform: scale(0.6);
      opacity: 0;
    }
    46%,
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
