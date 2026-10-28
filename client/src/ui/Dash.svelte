<script lang="ts">
  // The bar floating over a session's screen (the demo visitor's, and the administrator's on a session): who, where and
  // control on the left, the session on the right. It slides away after a few seconds without the pointer on it and
  // comes back when the pointer is on the handle at the top (or the handle is clicked or focused), as remote-desktop
  // bars do; `pinned` holds it out (an open menu, a recording under way).
  import { type Snippet, tick } from 'svelte';
  import { t } from '../i18n/index.svelte.ts';
  import { focusRing } from './ui.ts';

  let { label, pinned = false, lead, trail }: { label: string; pinned?: boolean; /** Its left zone. */ lead: Snippet; /** Its right zone. */ trail: Snippet } = $props();

  const HIDE_AFTER_MS = 3000;

  let shown = $state(true);
  let hovering = $state(false);
  // Bumped as focus leaves something in the bar, which restarts the wait (see below).
  let focusMoves = $state(0);
  let bar: HTMLElement | undefined = $state();
  const held = $derived(hovering || pinned);

  // Hidden after a while unless something holds it out: the pointer, keyboard focus, or `pinned`. Only keyboard focus
  // (`:focus-visible`) counts: Chrome focuses a clicked button, and that focus stays once the pointer leaves, so counting
  // any focus would keep the bar out after every click. Focus is read when the wait ends, not tracked: Firefox and
  // Safari fire no focusout when the focused element is removed (a demo in the list, say), which would hold the bar out
  // for good.
  $effect(() => {
    void focusMoves;
    if (!shown || held) return;
    const timer = setInterval(() => {
      if (!bar?.querySelector(':focus-visible')) shown = false;
    }, HIDE_AFTER_MS);
    return () => clearInterval(timer);
  });

  // From the keyboard, focus goes into the bar as it shows (the handle that had it disappears).
  async function reveal(focus: boolean) {
    shown = true;
    if (!focus) return;
    await tick();
    // The first button that takes focus: control may be disabled while it is being taken.
    bar?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
  }
</script>

<!-- The handle at the top: pointing at it brings the bar back. -->
{#if !shown}
  <button
    class="fixed left-1/2 top-0 z-30 flex h-4 w-14 -translate-x-1/2 items-center justify-center rounded-b-lg bg-ink/70 text-white shadow-md backdrop-blur hover:bg-ink {focusRing}"
    type="button"
    aria-label={t('session.bar.show')}
    onpointerenter={() => reveal(false)}
    onclick={() => reveal(true)}
    onfocus={() => reveal(true)}
  >
    <svg viewBox="0 0 24 24" class="size-3.5" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6" /></svg>
  </button>
{/if}

<header
  bind:this={bar}
  role="region"
  aria-label={label}
  class="fixed inset-x-3 top-3 z-20 mx-auto flex min-h-16 max-w-[76rem] flex-wrap items-center gap-x-3 gap-y-1 rounded-2xl border border-line bg-paper/95 py-2 pl-4 pr-2.5 shadow-[0_18px_44px_-16px_rgb(20_40_75/0.4)] backdrop-blur-md transition duration-150 {shown ? 'translate-y-0 opacity-100' : 'pointer-events-none -translate-y-[calc(100%+1rem)] opacity-0'}"
  inert={!shown}
  onpointerenter={() => (hovering = true)}
  onpointerleave={() => (hovering = false)}
  onfocusout={() => focusMoves++}
>
  <div class="flex items-center gap-3">{@render lead()}</div>
  <div class="ml-auto flex flex-wrap items-center gap-1">{@render trail()}</div>
</header>
