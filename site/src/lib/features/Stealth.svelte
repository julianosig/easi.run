<script lang="ts">
  // An ordinary Chrome, drawn: a bot detector's page lists its tests, and each comes back clean in turn (a check where
  // a warning could have been); then the verdict, a person's mark, shows. Decorative. It loops every 8 s, moving only
  // transforms and opacity; for less motion it stands still with every test passed.
  import Window from './Window.svelte';

  const ROWS = [40, 58, 76, 94];
</script>

<Window>
  {#each ROWS as y, i (y)}
    <rect x="40" y={y} width={[120, 96, 108, 88][i]} height="6" rx="3" class="fill-ink/15" />
    <g class="pass p{i}" transform="translate(186 {y + 3})">
      <circle r="6.5" class="fill-accent" />
      <path d="m-3 0 2.2 2.2 4-4.4" class="fill-none stroke-white" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" />
    </g>
  {/each}
  <!-- The verdict: a person. -->
  <g class="verdict">
    <circle cx="248" cy="68" r="26" class="fill-accent-soft" />
    <circle cx="248" cy="60" r="8" class="fill-accent" />
    <path d="M233 84a15 13 0 0 1 30 0z" class="fill-accent" />
  </g>
</Window>

<style>
  .pass > * {
    transform-box: fill-box;
    transform-origin: center;
    animation: pass 8s ease-out infinite;
  }
  .p1 > * {
    animation-delay: 0.6s;
  }
  .p2 > * {
    animation-delay: 1.2s;
  }
  .p3 > * {
    animation-delay: 1.8s;
  }
  .verdict {
    transform-box: fill-box;
    transform-origin: center;
    animation: verdict 8s ease-out infinite;
  }

  @keyframes pass {
    0%,
    8% {
      transform: scale(0);
      opacity: 0;
    }
    13% {
      transform: scale(1.2);
      opacity: 1;
    }
    16%,
    84% {
      transform: none;
      opacity: 1;
    }
    92%,
    100% {
      opacity: 0;
    }
  }
  @keyframes verdict {
    0%,
    40% {
      transform: scale(0.8);
      opacity: 0;
    }
    48%,
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
