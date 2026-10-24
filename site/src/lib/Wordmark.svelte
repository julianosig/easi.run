<script lang="ts">
  // The name as a mark: lowercase, with your coral dot. With `mark` (`'inverse'` on blue or dark grounds), the cursor
  // stands before it in the line of text, scaled with the name: its middle level with the middle of the short letters
  // (the x-height's, 0.27em over the baseline).
  import Logo from './Logo.svelte';

  let { class: className = '', mark = false }: { class?: string; mark?: boolean | 'inverse' } = $props();
</script>

<span class="whitespace-nowrap font-display font-semibold tracking-[-0.03em] {className}"
  >{#if mark}<Logo class="mr-[0.45em] inline-block h-[1.11em] w-auto align-[-0.28em]" inverse={mark === 'inverse'} />{/if}easi<span
    class="text-you">.</span
  >run</span
>
