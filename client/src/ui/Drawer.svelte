<script lang="ts">
  // A panel over the right of the page (a session's timeline, say): it takes the keyboard's focus as it opens, so keys
  // stop reaching the remote browser under it, Escape closes it, and the focus goes back where it was.
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import { ICONS } from './icons.ts';
  import { t } from '../i18n/index.svelte.ts';
  import { focusRing } from './ui.ts';

  let { title, icon, onClose, children }: { title: string; icon: string; onClose: () => void; children: Snippet } = $props();

  const heading = `drawer-${Math.random().toString(36).slice(2)}`;
  let closer: HTMLButtonElement | undefined = $state();
  $effect(() => {
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    closer?.focus();
    return () => before?.isConnected && before.focus();
  });
</script>

<!-- A dialog that leaves the page under it usable (not modal): the session goes on beside it. -->
<div
  class="fixed inset-y-0 right-0 z-40 flex w-full max-w-md flex-col bg-paper shadow-[-12px_0_40px_-20px_rgb(20_40_75/0.45)]"
  role="dialog"
  aria-modal="false"
  tabindex="-1"
  aria-labelledby={heading}
  onkeydown={(e) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onClose();
    }
  }}
>
  <header class="flex items-center justify-between gap-3 border-b border-line px-5 py-4">
    <h2 id={heading} class="flex min-w-0 items-center gap-2 text-[17px] font-semibold"><Icon d={icon} class="size-5 shrink-0 text-accent" /><span class="truncate">{title}</span></h2>
    <button bind:this={closer} class="grid size-9 shrink-0 place-items-center rounded-full text-muted hover:bg-wash hover:text-ink {focusRing}" type="button" aria-label={t('timeline.close')} title={t('timeline.close')} onclick={onClose}
      ><Icon d={ICONS.close} class="size-5" /></button
    >
  </header>
  <div class="min-h-0 flex-1 overflow-y-auto px-5 py-4">
    {@render children()}
  </div>
</div>
