<script lang="ts">
  // The easi.run mark: the agent's cursor in blue, and the coral dot that is you, ready to take over. `inverse` draws
  // the cursor white for blue or dark grounds. Decorative: the name always stands next to it. The view box is the
  // drawing's own bounds, so its height is the cursor's (the wordmark sizes it to the name's letters).
  let { class: className = 'h-7 w-auto', inverse = false }: { class?: string; inverse?: boolean } = $props();
</script>

<svg viewBox="10 4 48 56" class={className} aria-hidden="true">
  <path d="M12 6 56 30 35 36 25 58Z" class={inverse ? 'fill-white stroke-white' : 'fill-accent stroke-accent'} stroke-width="3.5" stroke-linejoin="round" />
  <circle cx="51" cy="51" r="6.5" class="fill-you" />
</svg>
