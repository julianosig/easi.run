<script lang="ts">
  // The name as a mark, as on the landing (site/src/lib/Wordmark.svelte): lowercase, with your coral dot, and the
  // cursor before it in the line of text, scaled with the name, its middle level with the middle of the short letters.
  import Mark from './Mark.svelte';

  let { class: className = '' }: { class?: string } = $props();
</script>

<span class="whitespace-nowrap font-semibold tracking-[-0.03em] {className}"
  ><Mark class="mr-[0.45em] inline-block h-[1.11em] w-auto align-[-0.28em]" />easi<span class="text-you">.</span>run</span
>
