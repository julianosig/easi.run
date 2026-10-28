<script lang="ts">
  // A page's head, the same on every page: its icon in a badge, its title and one plain line on the left, and on the
  // right, centred on them, the page's add: a large "+" alone, named by `add.label`, rings of light leaving it.
  import Button from './Button.svelte';
  import Icon from './Icon.svelte';
  import { ICONS } from './icons.ts';

  let {
    icon,
    title,
    subtitle,
    add,
  }: { icon: string; title: string; subtitle: string; add?: { label: string; onclick: () => void } | undefined } = $props();
</script>

<header class="flex items-center justify-between gap-6">
  <div class="flex min-w-0 items-center gap-4">
    <span class="hidden size-12 shrink-0 place-items-center rounded-2xl bg-accent-soft text-accent sm:grid"><Icon d={icon} class="size-6" /></span>
    <div class="min-w-0">
      <h1 class="text-[1.75rem] font-semibold leading-tight tracking-[-0.03em]">{title}</h1>
      <p class="mt-0.5 max-w-2xl text-sm leading-relaxed text-muted">{subtitle}</p>
    </div>
  </div>
  {#if add}
    <span class="relative grid shrink-0 place-items-center">
      <span class="wave" aria-hidden="true"></span>
      <span class="wave [animation-delay:1.4s]" aria-hidden="true"></span>
      <Button variant="primary" size="xl" icon={ICONS.add} label={add.label} onclick={add.onclick} class="relative" />
    </span>
  {/if}
</header>
