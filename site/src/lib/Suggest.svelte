<script lang="ts">
  // On an English page, for a visitor whose browser prefers another language the site speaks and who has picked none:
  // a note, in that language, offering its page. Never a redirect: / stays the English page for everyone who wants it.
  // For the landing and the docs alike: each gives its language and how to read the note's words in another.
  import { AUTONYMS, choose, chosen, type Locale, match, pathOf, SOURCE } from '../i18n/locale.ts';
  import { focus } from './focus.ts';
  import { currentHash } from './hash.svelte.ts';

  /** The note's words in `to`: what it says, its link (naming the language) and its dismissal. */
  export type Words = (to: Locale, language: string) => Promise<{ text: string; go: string; stay: string }>;
  let { current, words }: { current: Locale; words: Words } = $props();
  let offer: { to: Locale; text: string; go: string; stay: string } | null = $state(null);
  /** The section on screen, so the link lands on it in the other language. */
  const hash = currentHash();

  $effect(() => {
    if (current !== SOURCE || chosen() !== null) return;
    const to = match(navigator.languages ?? []);
    if (to === SOURCE) return;
    words(to, AUTONYMS[to]).then(
      (said) => (offer = { to, ...said }),
      () => undefined,
    );
  });

  function stay() {
    choose(SOURCE);
    offer = null;
  }
</script>

{#if offer}
  <div class="fixed inset-x-4 bottom-4 z-50 mx-auto flex max-w-xl flex-wrap items-center gap-x-4 gap-y-2 rounded-2xl border border-line bg-surface px-5 py-3 text-sm shadow-[0_24px_60px_-24px_rgb(20_40_75/0.45)]" lang={offer.to} role="region" aria-label={offer.text}>
    <p class="min-w-0 flex-1 text-muted">{offer.text}</p>
    <a href={pathOf(offer.to) + hash.value} class="rounded-xl bg-accent px-3.5 py-2 font-semibold text-white {focus}" onclick={() => offer && choose(offer.to)}>{offer.go}</a>
    <button type="button" class="rounded-xl px-2 py-2 font-semibold text-muted hover:text-ink {focus}" onclick={stay}>{offer.stay}</button>
  </div>
{/if}
