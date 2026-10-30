<script lang="ts">
  // On an English page, for a visitor whose browser prefers another language the site speaks and who has picked none:
  // a note, in that language, offering its page. Never a redirect: / stays the English page for everyone who wants it.
  // For the landing and the docs alike: each gives its language, its catalogs, and the area of the note's messages in
  // them (`<area>.suggest.text`, `.go` naming the language, `.dismiss`); a landing page, its own path.
  import { type Catalog, format } from '../../../i18n/format.ts';
  import { AUTONYMS, choose, chosen, type Locale, match, pathOf, SOURCE } from '../i18n/locale.ts';
  import { focus } from './focus.ts';
  import { currentHash } from './hash.svelte.ts';

  let {
    current,
    catalogs,
    area,
    page = '',
  }: { current: Locale; catalogs: Record<Locale, () => Promise<{ default: Catalog }>>; area: 'page' | 'shell'; page?: string } = $props();

  /** The note's words in `to`, from that language's catalog. */
  async function words(to: Locale, language: string): Promise<{ text: string; go: string; stay: string }> {
    const { default: catalog } = await catalogs[to]();
    const say = (name: string, params = {}) => format(catalog[`${area}.suggest.${name}`] ?? '', params, to);
    return { text: say('text'), go: say('go', { language }), stay: say('dismiss') };
  }
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
    <a href={pathOf(offer.to, page) + hash.value} class="rounded-xl bg-accent px-3.5 py-2 font-semibold text-white {focus}" onclick={() => offer && choose(offer.to)}>{offer.go}</a>
    <button type="button" class="rounded-xl px-2 py-2 font-semibold text-muted hover:text-ink {focus}" onclick={stay}>{offer.stay}</button>
  </div>
{/if}
