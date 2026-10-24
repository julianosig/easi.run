<script lang="ts">
  // The one loading state: a shimmer over the whole page, saying only that something is on its way. What is happening
  // is announced to screen readers.
  let { said }: { said: string } = $props();
</script>

<div class="shimmer fixed inset-0 z-50" role="status">
  <span class="sr-only">{said}</span>
</div>

<style>
  .shimmer {
    background: linear-gradient(110deg, var(--color-wash) 30%, var(--color-accent-soft) 45%, #ffffff 50%, var(--color-accent-soft) 55%, var(--color-wash) 70%);
    background-size: 250% 100%;
    animation: shimmer 1.6s linear infinite;
  }
  @keyframes shimmer {
    from {
      background-position: 100% 0;
    }
    to {
      background-position: -150% 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .shimmer {
      animation: none;
    }
  }
</style>
