<script lang="ts">
  // Recording, drawn: the session's window with a red dot pulsing beside it, your camera as a round bubble in its
  // corner, your microphone's level moving, and the recording's file growing as it streams to your Downloads.
  // Decorative. It loops every 4 s, moving only transforms and opacity; for less motion it stands still recording.
  import Window from './Window.svelte';
</script>

<Window>
  <rect x="40" y="36" width="96" height="8" rx="4" class="fill-ink/15" />
  <rect x="40" y="52" width="140" height="5" rx="2.5" class="fill-wash" />
  <rect x="40" y="63" width="124" height="5" rx="2.5" class="fill-wash" />
  <rect x="40" y="74" width="132" height="5" rx="2.5" class="fill-wash" />
  <!-- Recording: the dot on the toolbar. -->
  <circle cx="276" cy="15" r="4" class="rec fill-you" />
  <!-- The growing file. -->
  <rect x="40" y="96" width="120" height="6" rx="3" class="fill-line" />
  <rect x="40" y="96" width="120" height="6" rx="3" class="grow fill-you" />
  <!-- Your microphone. -->
  {#each [0, 1, 2, 3, 4] as i (i)}
    <rect x={176 + i * 8} y="88" width="4" height="20" rx="2" class="mic m{i} fill-accent" />
  {/each}
  <!-- Your camera, in the corner. -->
  <circle cx="254" cy="84" r="28" class="fill-accent-soft stroke-paper" stroke-width="3" />
  <circle cx="254" cy="76" r="9" class="fill-accent" />
  <path d="M236 106a18 16 0 0 1 36 0" class="fill-accent" />
</Window>

<style>
  .rec {
    animation: rec 1.6s ease-in-out infinite;
  }
  .grow {
    transform-box: fill-box;
    transform-origin: left center;
    transform: scaleX(0.7);
    animation: grow 4s linear infinite;
  }
  .mic {
    transform-box: fill-box;
    transform-origin: center;
    transform: scaleY(0.6);
    animation: mic 0.9s ease-in-out infinite alternate;
  }
  .m1,
  .m3 {
    animation-delay: -0.45s;
  }
  .m2 {
    animation-delay: -0.2s;
  }

  @keyframes rec {
    50% {
      opacity: 0.3;
    }
  }
  @keyframes grow {
    from {
      transform: scaleX(0.05);
    }
    to {
      transform: scaleX(1);
    }
  }
  @keyframes mic {
    from {
      transform: scaleY(0.3);
    }
    to {
      transform: scaleY(1);
    }
  }
</style>
