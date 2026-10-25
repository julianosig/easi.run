<script lang="ts">
  // A row's or a bar's further actions, behind one button: the list opens under it and closes on a pick, a press
  // outside it, or Escape (which puts focus back on the button). `label` names the button for screen readers.
  import type { Snippet } from 'svelte';
  import { dismissable } from './dismiss.ts';
  import { focusRing } from './ui.ts';

  let {
    label,
    open = $bindable(false),
    children,
  }: { label: string; open?: boolean; children: Snippet } = $props();

  let root: HTMLDivElement | undefined = $state();
  let trigger: HTMLButtonElement | undefined = $state();
  const list = `menu-${Math.random().toString(36).slice(2, 10)}`;
  $effect(() => {
    if (open) return dismissable(root, () => (open = false));
  });
</script>

<div class="relative" bind:this={root}>
  <button
    class="grid size-10 place-items-center rounded-full text-muted transition hover:bg-wash hover:text-ink {open ? 'bg-wash text-ink' : ''} {focusRing}"
    type="button"
    bind:this={trigger}
    aria-label={label}
    title={label}
    aria-expanded={open}
    aria-controls={list}
    onclick={() => (open = !open)}
  >
    <svg viewBox="0 0 24 24" class="size-5" fill="currentColor" aria-hidden="true"><circle cx="5.5" cy="12" r="1.7" /><circle cx="12" cy="12" r="1.7" /><circle cx="18.5" cy="12" r="1.7" /></svg>
  </button>
  {#if open}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
    <div
      id={list}
      class="absolute right-0 z-30 w-60 rounded-2xl border border-line bg-paper p-1.5 top-full mt-2 shadow-[0_24px_60px_-24px_rgb(20_40_75/0.45)]"
      role="group"
      aria-label={label}
      onclick={(e) => {
        // A pick takes the list away with the focused item: focus goes back to the button first, where a dialog the
        // pick opens returns it when it closes.
        if ((e.target as Element).closest('button')) trigger?.focus();
      }}
    >
      {@render children()}
    </div>
  {/if}
</div>
