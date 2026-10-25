<script lang="ts">
  // A line icon in 24 units, drawn in the text's colour as the landing's menu draws its own. Decorative.
  let { d, class: className = 'size-[18px]' }: { d: string; class?: string } = $props();
</script>

<svg viewBox="0 0 24 24" class={className} fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
  ><path {d} /></svg
>
