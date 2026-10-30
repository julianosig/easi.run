<script lang="ts">
  // Your own file picker, drawn: your cursor presses the page's upload button, your computer's own picker rises over
  // the window (its sidebar and files), a file is picked, and its name appears on the page. Decorative. It loops every
  // 8 s, moving only transforms and opacity; for less motion it stands still with the file chosen on the page.
  import Cursor from './Cursor.svelte';
  import Window from './Window.svelte';
</script>

<Window>
  <rect x="40" y="36" width="80" height="8" rx="4" class="fill-ink/15" />
  <rect x="40" y="56" width="70" height="20" rx="10" class="press fill-accent" />
  <path d="M75 70v-8m-3.5 3.5L75 62l3.5 3.5" class="fill-none stroke-white" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
  <g class="chosen">
    <path d="M120 58h9l4 4v12h-13z" class="fill-paper stroke-ink" stroke-width="1.5" stroke-linejoin="round" />
    <rect x="140" y="63" width="64" height="6" rx="3" class="fill-ink/25" />
  </g>

  <!-- Your computer's picker. -->
  <g class="picker">
    <rect x="116" y="30" width="168" height="86" rx="8" class="fill-paper stroke-ink/30" stroke-width="1.5" />
    <rect x="116" y="30" width="44" height="86" rx="8" class="fill-wash" />
    {#each [44, 56, 68] as y (y)}<rect x="124" y={y} width="28" height="4" rx="2" class="fill-ink/20" />{/each}
    {#each [0, 1, 2] as i (i)}
      <g transform="translate({172 + i * 36} 46)">
        <rect x="-4" y="-6" width="32" height="40" rx="5" class={i === 1 ? 'pick fill-accent-soft' : 'fill-none'} />
        <path d="M2 0h14l6 6v20H2z" class="fill-paper stroke-ink/60" stroke-width="1.5" stroke-linejoin="round" />
      </g>
    {/each}
    <rect x="240" y="100" width="36" height="10" rx="5" class="fill-accent" />
  </g>

  <g class="you"><Cursor you /></g>
</Window>

<style>
  .you {
    transform: translate(100px, 72px);
    animation: you 8s ease-in-out infinite;
  }
  .press {
    transform-box: fill-box;
    transform-origin: center;
    animation: press 8s ease-in-out infinite;
  }
  .picker {
    opacity: 0;
    animation: picker 8s cubic-bezier(0.3, 0, 0.2, 1) infinite;
  }
  .pick {
    animation: pick 8s step-end infinite;
  }
  .chosen {
    animation: chosen 8s ease-out infinite;
  }

  @keyframes you {
    0% {
      transform: translate(170px, 110px);
      opacity: 0;
    }
    6% {
      opacity: 1;
    }
    18% {
      transform: translate(84px, 70px) scale(1);
    }
    21% {
      transform: translate(84px, 70px) scale(0.85);
    }
    24% {
      transform: translate(84px, 70px) scale(1);
    }
    44% {
      transform: translate(222px, 74px) scale(1);
    }
    47% {
      transform: translate(222px, 74px) scale(0.85);
    }
    52% {
      transform: translate(222px, 74px) scale(1);
    }
    60% {
      transform: translate(260px, 108px) scale(1);
    }
    63% {
      transform: translate(260px, 108px) scale(0.85);
    }
    66%,
    90% {
      transform: translate(100px, 72px) scale(1);
      opacity: 1;
    }
    98%,
    100% {
      transform: translate(100px, 72px);
      opacity: 0;
    }
  }
  @keyframes press {
    0%,
    20% {
      transform: none;
    }
    22% {
      transform: scale(0.92);
    }
    25%,
    100% {
      transform: none;
    }
  }
  @keyframes picker {
    0%,
    24% {
      transform: translateY(14px);
      opacity: 0;
    }
    30%,
    62% {
      transform: none;
      opacity: 1;
    }
    68%,
    100% {
      transform: translateY(14px);
      opacity: 0;
    }
  }
  @keyframes pick {
    0% {
      opacity: 0;
    }
    46% {
      opacity: 1;
    }
    66% {
      opacity: 0;
    }
  }
  @keyframes chosen {
    0%,
    66% {
      transform: translateX(-6px);
      opacity: 0;
    }
    72%,
    92% {
      transform: none;
      opacity: 1;
    }
    98%,
    100% {
      opacity: 0;
    }
  }
</style>
