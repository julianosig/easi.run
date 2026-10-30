<svelte:options namespace="svg" />

<script lang="ts">
  // A pointer, as the agents' card first drew it: blue for an agent, coral for you. Its tip is at the origin, so a
  // drawing places it, and moves it, by a group around it.
  let { you = false }: { you?: boolean } = $props();
</script>

<path d="M12 6 56 30 35 36 25 58Z" transform="scale(0.42) translate(-12 -6)" class="{you ? 'fill-you' : 'fill-accent'} stroke-white" stroke-width="5" stroke-linejoin="round" />
