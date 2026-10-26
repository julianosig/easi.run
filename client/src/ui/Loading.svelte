<script lang="ts">
  // The one loading state: a shimmer over the whole page, saying only that something is on its way. What is happening
  // is announced to screen readers.
  let { said }: { said: string } = $props();
</script>

<div class="shimmer fixed inset-0 z-50" role="status">
  <span class="sr-only">{said}</span>
</div>

