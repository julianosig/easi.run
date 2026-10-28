<script lang="ts">
  // On the English page, for a visitor whose browser prefers another language the site speaks and who has picked none:
  // a note, in that language, offering its page. Never a redirect: / stays the English page for everyone who wants it.
  import { format } from '../../../i18n/format.ts';
  import { AUTONYMS, CATALOGS, choose, chosen, locale, match, pathOf, SOURCE, type Locale } from '../i18n/index.ts';
  import { focus } from './focus.ts';

  let offer: { to: Locale; text: string; go: string; stay: string } | null = $state(null);

  $effect(() => {
    if (locale() !== SOURCE || chosen() !== null) return;
    const to = match(navigator.languages ?? []);
    if (to === SOURCE) return;
    CATALOGS[to]().then(
      ({ default: catalog }) => {
        const say = (key: string, params = {}) => format(catalog[key] ?? '', params, to);
        offer = { to, text: say('page.suggest.text'), go: say('page.suggest.go', { language: AUTONYMS[to] }), stay: say('page.suggest.dismiss') };
      },
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
    <a href={pathOf(offer.to) + location.hash} class="rounded-xl bg-accent px-3.5 py-2 font-semibold text-white {focus}" onclick={() => offer && choose(offer.to)}>{offer.go}</a>
    <button type="button" class="rounded-xl px-2 py-2 font-semibold text-muted hover:text-ink {focus}" onclick={stay}>{offer.stay}</button>
  </div>
{/if}
