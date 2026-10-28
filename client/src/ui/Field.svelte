<script lang="ts">
  // A form's field: the control with its icon inside, at its start, a word or two of placeholder in it, and under it one
  // plain line on what to put there. Its name is for screen readers only (the icon and placeholder say it on screen).
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  let { icon, label, for: id, help = '', children }: { icon: string; label: string; for: string; help?: string; children: Snippet } = $props();
</script>

<div class="mt-5">
  <label class="sr-only" for={id}>{label}</label>
  <div class="group relative [&>input]:pl-11 [&>select]:pl-11">
    <Icon d={icon} class="pointer-events-none absolute left-4 top-1/2 size-[18px] -translate-y-1/2 text-muted transition group-focus-within:text-accent" />
    {@render children()}
  </div>
  {#if help}<p id="{id}-help" class="mt-1.5 text-[13px] text-muted">{help}</p>{/if}
</div>
