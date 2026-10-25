<script lang="ts">
  // The top bar, as the landing's (site/src/lib/Nav.svelte): a floating bar that stays at the top of the page, with
  // the mark and the name, the view's actions as icon-and-label buttons, and the log out button last, shaped as the
  // landing's repository button. On a narrow screen the buttons show their icons alone; their labels stay their names.
  import { focusRing } from './ui.ts';
  import Wordmark from './Wordmark.svelte';

  let {
    actions = [],
    onLogout,
    class: className = '',
  }: {
    /** Each with its icon: a path in 24 units, drawn as a 1.8-unit stroke. */
    actions?: { label: string; icon: string; onclick: () => void }[];
    onLogout?: () => void;
    class?: string;
  } = $props();
  const LOG_OUT = 'M14 4h4a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-4M10 8l-4 4 4 4M6 12h10';
</script>

{#snippet icon(path: string, tint: string)}
  <svg viewBox="0 0 24 24" class="size-[18px] shrink-0 {tint}" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
    ><path d={path} /></svg
  >
{/snippet}

<header class="sticky top-3 z-40 {className}">
  <nav
    class="mx-auto flex h-16 max-w-[76rem] items-center justify-between gap-2 rounded-2xl border border-line bg-paper/90 pl-5 pr-3 shadow-[0_10px_34px_-14px_rgb(20_40_75/0.28)] backdrop-blur-md sm:gap-4"
    aria-label="Main"
  >
    <Wordmark class="text-[22px]" />
    {#if actions.length}
      <ul class="flex items-center gap-1">
        {#each actions as action (action.label)}
          <li>
            <button
              class="flex items-center gap-2 rounded-xl px-3 py-2 text-[15px] font-semibold text-ink/75 transition hover:bg-wash hover:text-ink sm:px-3.5 {focusRing}"
              type="button"
              onclick={action.onclick}
              >{@render icon(action.icon, 'text-accent/70')}<span class="sr-only sm:not-sr-only">{action.label}</span></button
            >
          </li>
        {/each}
      </ul>
    {/if}
    {#if onLogout}
      <button
        class="inline-flex items-center gap-2 rounded-xl bg-paper px-3 py-2 text-[15px] font-semibold text-ink ring-1 ring-line transition hover:ring-ink/30 {focusRing}"
        type="button"
        onclick={onLogout}>{@render icon(LOG_OUT, 'text-ink/70')}<span class="sr-only sm:not-sr-only">Log out</span></button
      >
    {/if}
  </nav>
</header>
