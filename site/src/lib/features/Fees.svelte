<script lang="ts">
  // No usage fees, drawn: a chart of cost over time; as the hours pass (the clock's hand turning), a metered service's
  // bill climbs, while mine stays flat at the price of the server. Decorative. It loops every 8 s, moving only
  // transforms and opacity; for less motion it stands still with both lines drawn.
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The clock. -->
  <circle cx="52" cy="64" r="26" class="fill-paper stroke-ink" stroke-width="2.5" />
  <path d="M52 64V48" class="hand fill-none stroke-you" stroke-width="2.5" stroke-linecap="round" />
  <path d="M52 64h10" class="fill-none stroke-ink" stroke-width="2.5" stroke-linecap="round" />

  <!-- The chart: the metered bill climbing, mine flat; drawn as time passes (a cover of the card's own colour moves off
       them), then its axes. -->
  <path d="M104 100l40-12 40-14 40-18 40-20 36-20" class="fill-none stroke-muted/60" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" stroke-dasharray="6 5" />
  <path d="M104 92h196" class="fill-none stroke-accent" stroke-width="4" stroke-linecap="round" />
  <rect x="102" y="10" width="208" height="98" class="cover fill-wash" />
  <path d="M100 18v92h204" class="fill-none stroke-line" stroke-width="2" stroke-linecap="round" />
</svg>

<style>
  .hand {
    transform-origin: 52px 64px;
    animation: hand 8s linear infinite;
  }
  .cover {
    transform: translateX(212px);
    animation: cover 8s ease-in-out infinite;
  }

  @keyframes hand {
    to {
      transform: rotate(720deg);
    }
  }
  @keyframes cover {
    0%,
    4% {
      transform: none;
    }
    60%,
    100% {
      transform: translateX(212px);
    }
  }
</style>
