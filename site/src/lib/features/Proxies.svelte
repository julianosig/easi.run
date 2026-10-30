<script lang="ts">
  // Your own proxy, drawn: the browser on the left, its traffic going through your proxy (the middle node) out to where
  // it exits (a pin on the map); when it lands, the browser's clock turns to that place's time. Decorative. It loops
  // every 8 s, moving only transforms and opacity; for less motion it stands still on the exit's time.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The browser, and its clock. -->
  <rect x="16" y="36" width="72" height="56" rx="8" class="fill-paper stroke-line" stroke-width="1.5" />
  <path d="M24 36h56a8 8 0 0 1 8 8v4H16v-4a8 8 0 0 1 8-8z" class="fill-accent-soft" />
  <circle cx="52" cy="72" r="13" class="fill-paper stroke-ink" stroke-width="2" />
  <path d="M52 72v-8" class="minute fill-none stroke-ink" stroke-width="2" stroke-linecap="round" />
  <path d="M52 72l5 3" class="hour fill-none stroke-you" stroke-width="2.5" stroke-linecap="round" />

  <!-- The way out, through the proxy. -->
  <path d="M92 64h52M176 64h52" class="fill-none stroke-accent/30" stroke-width="2" stroke-dasharray="4 4" />
  <circle cx="160" cy="64" r="15" class="fill-accent" />
  <path d="M153 64h14m-4-4 4 4-4 4" class="fill-none stroke-white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
  <circle cx="96" cy="64" r="4" class="packet fill-accent" />

  <!-- Where it exits: a map, and its pin. -->
  <rect x="232" y="30" width="72" height="68" rx="10" class="fill-wash" />
  <path d="M240 56c10-6 18 2 26-2s16-8 30 0M240 78c12-4 20 4 30 0s14-6 26-2" class="fill-none stroke-line" stroke-width="2" />
  <g class="pin">
    <path d="M268 74c-8-9-12-15-12-21a12 12 0 0 1 24 0c0 6-4 12-12 21z" class="fill-you" />
    <circle cx="268" cy="53" r="4.5" class="fill-white" />
  </g>
</svg>

<style>
  .packet {
    opacity: 0;
    animation: packet 8s ease-in-out infinite;
  }
  .pin {
    transform-box: fill-box;
    transform-origin: 50% 100%;
    animation: pin 8s ease-out infinite;
  }
  .hour {
    transform-origin: 52px 72px;
    transform: rotate(120deg);
    animation: hour 8s ease-in-out infinite;
  }

  @keyframes packet {
    0%,
    4% {
      transform: none;
      opacity: 0;
    }
    8% {
      opacity: 1;
    }
    40% {
      transform: translateX(160px);
      opacity: 1;
    }
    44%,
    100% {
      transform: translateX(160px);
      opacity: 0;
    }
  }
  @keyframes pin {
    0%,
    38% {
      transform: translateY(-14px);
      opacity: 0;
    }
    46% {
      transform: none;
      opacity: 1;
    }
    50% {
      transform: scaleY(0.9);
    }
    54%,
    92% {
      transform: none;
      opacity: 1;
    }
    100% {
      opacity: 0;
    }
  }
  @keyframes hour {
    0%,
    48% {
      transform: none;
    }
    62%,
    94% {
      transform: rotate(120deg);
    }
    100% {
      transform: none;
    }
  }
</style>
