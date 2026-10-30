<script lang="ts">
  // Crisp text and smooth motion, drawn: a page whose text stays still and sharp while a region of it moves (a video,
  // its frames sliding), outlined as the part sent as video; then the motion stops and the outline goes: a still page
  // sends nothing. Decorative. It loops every 8 s, moving only transforms and opacity; for less motion it stands still
  // with the region outlined.
  import Window from './Window.svelte';
</script>

<Window>
  <defs>
    <clipPath id="live-region"><rect x="176" y="34" width="104" height="72" rx="6" /></clipPath>
  </defs>
  <rect x="40" y="36" width="96" height="9" rx="4.5" class="fill-ink/80" />
  <rect x="40" y="54" width="116" height="5" rx="2.5" class="fill-ink/25" />
  <rect x="40" y="65" width="104" height="5" rx="2.5" class="fill-ink/25" />
  <rect x="40" y="76" width="112" height="5" rx="2.5" class="fill-ink/25" />
  <rect x="40" y="87" width="74" height="5" rx="2.5" class="fill-ink/25" />
  <rect x="40" y="100" width="92" height="5" rx="2.5" class="fill-ink/25" />

  <!-- The moving part: frames of a video passing. -->
  <rect x="176" y="34" width="104" height="72" rx="6" class="fill-void" />
  <g clip-path="url(#live-region)">
    <g class="frames">
      <path d="M176 106l30-34 22 22 18-16 34 28z" class="fill-accent/70" />
      <path d="M280 106l30-34 22 22 18-16 34 28z" class="fill-accent/70" />
      <circle cx="252" cy="54" r="8" class="fill-you/80" />
      <circle cx="356" cy="54" r="8" class="fill-you/80" />
    </g>
  </g>
  <rect x="171" y="29" width="114" height="82" rx="9" class="region fill-none stroke-accent" stroke-width="1.5" stroke-dasharray="5 4" />
</Window>

<style>
  .frames {
    animation: frames 8s linear infinite;
  }
  .region {
    animation: region 8s ease-in-out infinite;
  }

  @keyframes frames {
    0% {
      transform: none;
    }
    70%,
    100% {
      transform: translateX(-104px);
    }
  }
  @keyframes region {
    0%,
    66% {
      opacity: 1;
    }
    74%,
    94% {
      opacity: 0;
    }
    100% {
      opacity: 1;
    }
  }
</style>
