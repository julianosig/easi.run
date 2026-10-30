<script lang="ts">
  // The page's sound, drawn: a speaker sends out waves, one after another, while the levels beside it rise and fall,
  // and the volume's knob sits on its slider. Decorative. It loops every 2.4 s, moving only transforms and opacity; for
  // less motion it stands still with its waves and levels shown.
  const BARS = [0, 1, 2, 3, 4, 5, 6];
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The speaker. -->
  <path d="M60 52h14l18-15v54L74 76H60z" class="fill-ink" stroke-linejoin="round" />
  {#each [1, 2, 3] as i (i)}
    <path d="M{92 + i * 12} {64 - i * 11}a{i * 13} {i * 13} 0 0 1 0 {i * 22}" class="wave w{i} fill-none stroke-accent" stroke-width="3" stroke-linecap="round" />
  {/each}

  <!-- The levels. -->
  {#each BARS as i (i)}
    <rect x={176 + i * 14} y="34" width="8" height="60" rx="4" class="level v{i} fill-accent" />
  {/each}

  <!-- The volume. -->
  <rect x="176" y="106" width="92" height="4" rx="2" class="fill-line" />
  <rect x="176" y="106" width="64" height="4" rx="2" class="fill-accent" />
  <circle cx="240" cy="108" r="6" class="fill-paper stroke-accent" stroke-width="2" />
</svg>

<style>
  .wave {
    animation: wave 2.4s ease-out infinite;
  }
  .w2 {
    animation-delay: 0.3s;
  }
  .w3 {
    animation-delay: 0.6s;
  }
  .level {
    transform-box: fill-box;
    transform-origin: 50% 100%;
    transform: scaleY(0.6);
    animation: level 1.2s ease-in-out infinite alternate;
  }
  .v0,
  .v4 {
    animation-delay: -0.3s;
  }
  .v1,
  .v5 {
    animation-delay: -0.8s;
  }
  .v2 {
    animation-delay: -0.5s;
  }
  .v3,
  .v6 {
    animation-delay: -1.1s;
  }

  @keyframes wave {
    0% {
      opacity: 0.15;
    }
    30% {
      opacity: 1;
    }
    100% {
      opacity: 0.15;
    }
  }
  @keyframes level {
    from {
      transform: scaleY(0.25);
    }
    to {
      transform: scaleY(1);
    }
  }
</style>
