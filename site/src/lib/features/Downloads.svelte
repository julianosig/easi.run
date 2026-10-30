<script lang="ts">
  // Downloads, drawn: a file leaves the page and travels to your Downloads (a tray with its arrow), its progress filling
  // as it goes, byte by byte; the tray bounces once it is in. Decorative. It loops every 7 s, moving only transforms and
  // opacity; for less motion it stands still with the file in your Downloads.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The page, small, on the left. -->
  <rect x="20" y="18" width="128" height="92" rx="9" class="fill-paper stroke-line" stroke-width="1.5" />
  <path d="M29 18h110a9 9 0 0 1 9 9v5H20v-5a9 9 0 0 1 9-9z" class="fill-accent-soft" />
  <rect x="32" y="44" width="60" height="7" rx="3.5" class="fill-ink/15" />
  <rect x="32" y="58" width="96" height="5" rx="2.5" class="fill-wash" />
  <rect x="32" y="68" width="84" height="5" rx="2.5" class="fill-wash" />
  <rect x="32" y="86" width="44" height="12" rx="6" class="fill-accent" />

  <!-- The way out, and the progress along it. -->
  <path d="M160 64h70" class="fill-none stroke-line" stroke-width="4" stroke-linecap="round" />
  <path d="M160 64h70" class="progress fill-none stroke-accent" stroke-width="4" stroke-linecap="round" />

  <!-- Your Downloads. -->
  <g class="tray">
    <path d="M246 76v18a6 6 0 0 0 6 6h36a6 6 0 0 0 6-6V76" class="fill-none stroke-ink" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" />
    <path d="M270 34v40m-11-11 11 11 11-11" class="fill-none stroke-accent" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" />
  </g>

  <!-- The file. -->
  <g class="file">
    <path d="M262 82h12l6 6v12h-18z" class="fill-paper stroke-ink" stroke-width="1.6" stroke-linejoin="round" />
    <rect x="265" y="92" width="10" height="2.5" rx="1.25" class="fill-accent" />
  </g>
</svg>

<style>
  .progress {
    transform-box: fill-box;
    transform-origin: left center;
    animation: progress 7s ease-in-out infinite;
  }
  .file {
    animation: file 7s ease-in-out infinite;
  }
  .tray {
    animation: tray 7s ease-out infinite;
  }

  @keyframes progress {
    0%,
    8% {
      transform: scaleX(0);
      opacity: 1;
    }
    52%,
    86% {
      transform: none;
      opacity: 1;
    }
    94%,
    100% {
      transform: none;
      opacity: 0;
    }
  }
  @keyframes file {
    0%,
    6% {
      transform: translate(-150px, -10px);
      opacity: 0;
    }
    12% {
      opacity: 1;
    }
    52% {
      transform: translate(-10px, -20px);
    }
    60%,
    88% {
      transform: none;
      opacity: 1;
    }
    96%,
    100% {
      transform: none;
      opacity: 0;
    }
  }
  @keyframes tray {
    0%,
    58% {
      transform: none;
    }
    63% {
      transform: translateY(3px);
    }
    68%,
    100% {
      transform: none;
    }
  }
</style>
