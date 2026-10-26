<script lang="ts">
  // A page's main button: a plus, and what it makes.
  import { PLUS_ICON } from './admin.ts';
  import Icon from './Icon.svelte';
  import { primary } from './ui.ts';

  let { label, onclick, class: className = '' }: { label: string; onclick: () => void; class?: string } = $props();
</script>

<button class="{primary} inline-flex items-center gap-2 {className}" type="button" {onclick}><Icon d={PLUS_ICON} class="size-4" />{label}</button>
