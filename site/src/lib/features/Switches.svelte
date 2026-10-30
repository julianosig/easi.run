<script lang="ts">
  // Switching features per session, drawn: a panel of a session's features (input, clipboard, files, sound), each with
  // its switch; two flip off in turn and back on, the server's shield beside them holding each choice. Decorative. It
  // loops every 8 s, moving only transforms and opacity; for less motion it stands still with clipboard and files off.
  const ROWS = [
    { y: 26, d: 'M-6 -4h12v8h-12zM-3 -1h1M0 -1h1M3 -1h1M-3 2h7' },
    { y: 50, d: 'M-4 -5h8v10h-8zM-2 -7h4v3h-4z' },
    { y: 74, d: 'M-4 -6h5l3 3v9h-8z' },
    { y: 98, d: 'M-6 -2h3l4 -4v12l-4 -4h-3zM3 -3a4 4 0 0 1 0 6' },
  ];
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <rect x="56" y="10" width="208" height="108" rx="12" class="fill-paper stroke-line" stroke-width="1.5" />
  {#each ROWS as row, i (row.y)}
    <g transform="translate(80 {row.y + 2})"><path d={row.d} class="fill-none stroke-muted" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></g>
    <rect x="98" y={row.y - 1} width={[64, 76, 56, 48][i]} height="6" rx="3" class="fill-ink/15" />
    <!-- The switch: its track on, its track off over it, and its knob. -->
    <rect x="212" y={row.y - 8} width="36" height="20" rx="10" class="fill-accent" />
    <rect x="212" y={row.y - 8} width="36" height="20" rx="10" class="off o{i} fill-line" />
    <circle cx="238" cy={row.y + 2} r="7" class="knob k{i} fill-white" />
  {/each}
</svg>

<style>
  .off {
    opacity: 0;
    animation: off 8s ease-in-out infinite;
  }
  .knob {
    animation: knob 8s ease-in-out infinite;
  }
  .o0,
  .k0,
  .o3,
  .k3 {
    animation: none;
  }
  /* The still frame: clipboard and files off. */
  .o1,
  .o2 {
    opacity: 1;
  }
  .k1,
  .k2 {
    transform: translateX(-16px);
  }
  .o2,
  .k2 {
    animation-delay: 0.8s;
  }

  @keyframes off {
    0%,
    16% {
      opacity: 0;
    }
    24%,
    70% {
      opacity: 1;
    }
    78%,
    100% {
      opacity: 0;
    }
  }
  @keyframes knob {
    0%,
    16% {
      transform: none;
    }
    24%,
    70% {
      transform: translateX(-16px);
    }
    78%,
    100% {
      transform: none;
    }
  }
</style>
