<script lang="ts">
  // Drag and drop, drawn: your cursor carries a file in from your computer, the page's drop zone lights up, the file
  // drops in, and a progress bar fills right there to its check. Decorative. It loops every 7 s, moving only transforms
  // and opacity; for less motion it stands still on the dropped file, uploaded.
  import Cursor from './Cursor.svelte';
  import Window from './Window.svelte';
</script>

<Window>
  <rect x="40" y="36" width="72" height="8" rx="4" class="fill-ink/15" />
  <!-- The drop zone, and its light when a file is over it. -->
  <rect x="40" y="54" width="240" height="56" rx="10" class="fill-wash stroke-line" stroke-width="1.5" stroke-dasharray="6 5" />
  <rect x="40" y="54" width="240" height="56" rx="10" class="over fill-accent-soft stroke-accent" stroke-width="1.5" stroke-dasharray="6 5" />
  <!-- Where it lands: its name and progress. -->
  <g class="landed">
    <rect x="132" y="66" width="56" height="5" rx="2.5" class="fill-ink/25" />
    <rect x="132" y="78" width="112" height="5" rx="2.5" class="fill-line" />
    <rect x="132" y="78" width="112" height="5" rx="2.5" class="bar fill-accent" />
    <g class="done">
      <circle cx="256" cy="80" r="7" class="fill-accent" />
      <path d="m252.5 80 2.5 2.5 4.5-5" class="fill-none stroke-white" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
    </g>
  </g>
  <!-- The file, carried by your cursor. -->
  <g class="file">
    <path d="M100 62h16l8 8v22h-24z" class="fill-paper stroke-ink" stroke-width="1.8" stroke-linejoin="round" />
    <path d="M116 62v8h8" class="fill-none stroke-ink" stroke-width="1.8" stroke-linejoin="round" />
    <rect x="104" y="78" width="14" height="3" rx="1.5" class="fill-accent" />
    <rect x="104" y="84" width="10" height="3" rx="1.5" class="fill-ink/25" />
    <g transform="translate(118 86)"><Cursor you /></g>
  </g>
</Window>

<style>
  .file {
    animation: file 7s ease-in-out infinite;
  }
  .over {
    opacity: 0;
    animation: over 7s ease-in-out infinite;
  }
  .landed {
    animation: landed 7s ease-out infinite;
  }
  .bar {
    transform-box: fill-box;
    transform-origin: left center;
    animation: bar 7s ease-in-out infinite;
  }
  .done {
    transform-box: fill-box;
    transform-origin: center;
    animation: done 7s ease-out infinite;
  }

  @keyframes file {
    0% {
      transform: translate(200px, 50px);
      opacity: 0;
    }
    6% {
      opacity: 1;
    }
    28% {
      transform: none;
      opacity: 1;
    }
    34%,
    90% {
      transform: none;
      opacity: 1;
    }
    96%,
    100% {
      transform: none;
      opacity: 0;
    }
  }
  @keyframes over {
    0%,
    18% {
      opacity: 0;
    }
    24%,
    32% {
      opacity: 1;
    }
    38%,
    100% {
      opacity: 0;
    }
  }
  @keyframes landed {
    0%,
    32% {
      opacity: 0;
    }
    38%,
    90% {
      opacity: 1;
    }
    96%,
    100% {
      opacity: 0;
    }
  }
  @keyframes bar {
    0%,
    36% {
      transform: scaleX(0);
    }
    64%,
    100% {
      transform: none;
    }
  }
  @keyframes done {
    0%,
    64% {
      transform: scale(0);
    }
    68% {
      transform: scale(1.2);
    }
    72%,
    100% {
      transform: none;
    }
  }
</style>
