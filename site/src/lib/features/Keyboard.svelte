<script lang="ts">
  // Any keyboard, drawn: keys with characters from several languages, and the Mac's ⌘, pressed one after another, each
  // character landing in the field above as text. Decorative. It loops every 6 s, moving only transforms and opacity;
  // for less motion it stands still with the field typed.
  const KEYS = ['é', 'ñ', 'ü', 'ç', '⌘'];
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The field, and what lands in it. -->
  <rect x="60" y="14" width="200" height="30" rx="8" class="fill-paper stroke-accent" stroke-width="1.5" />
  {#each KEYS.slice(0, 4) as key, i (key)}
    <text x={78 + i * 18} y="35" class="typed c{i} fill-ink font-sans text-[16px] font-semibold">{key}</text>
  {/each}
  <rect x="152" y="21" width="2" height="16" rx="1" class="caret fill-accent" />

  <!-- The keys. -->
  {#each KEYS as key, i (key)}
    <g class="key k{i}">
      <rect x={42 + i * 48} y="60" width="40" height="40" rx="8" class="fill-ink/15" />
      <rect x={42 + i * 48} y="56" width="40" height="40" rx="8" class="fill-paper stroke-line" stroke-width="1.5" />
      <text x={62 + i * 48} y="82" text-anchor="middle" class="fill-ink font-sans text-[17px] font-semibold">{key}</text>
    </g>
  {/each}
</svg>

<style>
  .key {
    animation: key 6s ease-in-out infinite;
  }
  .typed {
    animation: typed 6s step-end infinite;
  }
  .k1,
  .c1 {
    animation-delay: 0.5s;
  }
  .k2,
  .c2 {
    animation-delay: 1s;
  }
  .k3,
  .c3 {
    animation-delay: 1.5s;
  }
  .k4 {
    animation-delay: 2.4s;
  }
  .caret {
    animation: caret 1s step-end infinite;
  }

  @keyframes key {
    0%,
    10% {
      transform: none;
    }
    13% {
      transform: translateY(3px);
    }
    17%,
    100% {
      transform: none;
    }
  }
  @keyframes typed {
    0% {
      opacity: 0;
    }
    13% {
      opacity: 1;
    }
    88% {
      opacity: 0;
    }
  }
  @keyframes caret {
    50% {
      opacity: 0;
    }
  }
</style>
