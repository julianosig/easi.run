<script lang="ts">
  // A form's field: its icon and label, the control, and one plain line on what it is for.
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  let { icon, label, for: id, help = '', children }: { icon: string; label: string; for: string; help?: string; children: Snippet } = $props();
</script>

<div class="mt-5">
  <label class="mb-1.5 flex items-center gap-2 text-sm font-semibold" for={id}><Icon d={icon} class="shrink-0 size-[18px] text-accent" />{label}</label>
  {@render children()}
  {#if help}<p id="{id}-help" class="mt-1.5 text-[13px] text-muted">{help}</p>{/if}
</div>
