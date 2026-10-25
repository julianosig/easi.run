<script lang="ts">
  // A page's head, the same on every page: its icon in a badge, its title and one plain line on the left, its main
  // action on the right.
  import type { Snippet } from 'svelte';
  import { pageHelp, pageTitle } from './admin.ts';
  import Icon from './Icon.svelte';

  let { icon, title, subtitle, action }: { icon: string; title: string; subtitle: string; action?: Snippet } = $props();
</script>

<header class="flex flex-wrap items-center justify-between gap-x-6 gap-y-4">
  <div class="flex min-w-0 items-center gap-4">
    <span class="hidden size-12 shrink-0 place-items-center rounded-2xl bg-accent-soft text-accent sm:grid"><Icon d={icon} class="size-6" /></span>
    <div class="min-w-0">
      <h1 class={pageTitle}>{title}</h1>
      <p class={pageHelp.replace('mt-1.5', 'mt-0.5')}>{subtitle}</p>
    </div>
  </div>
  {@render action?.()}
</header>
