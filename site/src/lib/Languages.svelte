<script lang="ts">
  // The page in another language: a link to each one's own address, named in itself (it works with no script). A pick
  // is remembered, so the site stops suggesting one (Suggest.svelte), and the section on screen comes along. For the
  // landing and the docs alike: each gives its language and its word for "Language", and a landing page its own path.
  import { AUTONYMS, choose, LOCALES, type Locale, pathOf } from '../i18n/locale.ts';
  import { focus } from './focus.ts';
  import { currentHash } from './hash.svelte.ts';

  let { current, label, page = '' }: { current: Locale; label: string; page?: string } = $props();

  let menu: HTMLDetailsElement | undefined = $state();
  /** The section on screen, so each link (opened here or in a new tab) lands on it in the other language. */
  const hash = currentHash();
  // A press outside it or Escape closes it.
  function onPointer(event: PointerEvent) {
    if (menu?.open && !menu.contains(event.target as Node)) menu.open = false;
  }
  function onKey(event: KeyboardEvent) {
    if (menu?.open && event.key === 'Escape') {
      menu.open = false;
      menu.querySelector('summary')?.focus();
    }
  }
</script>

<svelte:window onpointerdown={onPointer} onkeydown={onKey} />

<details class="relative" bind:this={menu}>
  <summary class="flex cursor-pointer list-none items-center gap-1.5 rounded-xl px-2.5 py-2 text-[15px] font-semibold text-ink/75 transition hover:bg-wash hover:text-ink {focus} [&::-webkit-details-marker]:hidden">
    <svg viewBox="0 0 24 24" class="size-[18px] text-accent/70" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
      ><circle cx="12" cy="12" r="9" /><path d="M3 12h18M12 3c2.5 2.7 3.8 5.7 3.8 9s-1.3 6.3-3.8 9c-2.5-2.7-3.8-5.7-3.8-9S9.5 5.7 12 3z" /></svg
    >
    <span aria-hidden="true">{current.slice(0, 2).toUpperCase()}</span>
    <span class="sr-only">{label}: {AUTONYMS[current]}</span>
  </summary>
  <ul class="absolute right-0 top-full z-50 mt-2 w-56 rounded-2xl border border-line bg-surface p-1.5 shadow-[0_24px_60px_-24px_rgb(20_40_75/0.45)]">
    {#each LOCALES as other (other)}
      <li>
        <a
          href={pathOf(other, page) + hash.value}
          hreflang={other}
          lang={other}
          class="block rounded-lg px-3 py-2 text-sm transition {focus} {other === current ? 'bg-accent-soft font-semibold text-accent' : 'text-ink/80 hover:bg-wash hover:text-ink'}"
          aria-current={other === current ? 'page' : undefined}
          onclick={() => choose(other)}>{AUTONYMS[other]}</a
        >
      </li>
    {/each}
  </ul>
</details>
