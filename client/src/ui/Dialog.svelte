<script lang="ts">
  // A panel over the page, for a task the page keeps hidden until it is needed (a form, a session's details): the
  // browser's modal dialog, which keeps focus inside it and the page behind it out of reach. Escape or its close button
  // closes it, and focus goes back to what opened it.
  import type { Snippet } from 'svelte';
  import { t } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';
  import { focusRing } from './ui.ts';

  let {
    open = $bindable(false),
    title,
    icon = '',
    subtitle = '',
    children,
  }: { open?: boolean; title: string; /** A line icon by its title (see admin.ts). */ icon?: string; subtitle?: string; children: Snippet } = $props();

  let dialog: HTMLDialogElement | undefined = $state();
  const heading = `dialog-${Math.random().toString(36).slice(2, 10)}`;

  $effect(() => {
    const d = dialog;
    if (!d || !open) return;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    d.showModal();
    // Into a form's first field, when it has one; its close button otherwise.
    d.querySelector<HTMLElement>('input:not([type=checkbox]), select, textarea')?.focus();
    return () => {
      if (d.open) d.close();
      // Back to what opened it, unless it went away meanwhile.
      if (opener?.isConnected) opener.focus();
    };
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby={heading}
  aria-describedby={subtitle ? `${heading}-sub` : undefined}
  class="m-auto max-h-[calc(100dvh-2rem)] w-[min(32rem,calc(100vw-2rem))] overflow-y-auto rounded-2xl border border-line bg-paper p-6 text-ink shadow-[0_30px_80px_-30px_rgb(20_40_75/0.55)] backdrop:bg-ink/25 backdrop:backdrop-blur-[2px] sm:p-8"
  oncancel={(e) => {
    e.preventDefault();
    open = false;
  }}
>
  {#if open}
    <div class="flex items-start gap-3.5">
      {#if icon}<span class="grid size-11 shrink-0 place-items-center rounded-2xl bg-accent-soft text-accent"><Icon d={icon} class="shrink-0 size-[22px]" /></span>{/if}
      <div class="min-w-0 flex-1 {icon ? 'pt-0.5' : ''}">
        <h2 id={heading} class="text-xl font-semibold tracking-[-0.02em]">{title}</h2>
        {#if subtitle}<p id="{heading}-sub" class="mt-0.5 text-sm text-muted">{subtitle}</p>{/if}
      </div>
      <button
        class="-mr-2 -mt-1 grid size-9 shrink-0 place-items-center rounded-full text-muted transition hover:bg-wash hover:text-ink {focusRing}"
        type="button"
        aria-label={t('common.close')}
        onclick={() => (open = false)}
      >
        <svg viewBox="0 0 24 24" class="size-5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" /></svg>
      </button>
    </div>
    {@render children()}
  {/if}
</dialog>
