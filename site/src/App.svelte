<script lang="ts">
  // The landing's frame, the same on each of its pages: the page's address in every language, the menu, the page, and
  // the name written large at the end.
  import Nav from './lib/Nav.svelte';
  import { NAME, ORIGIN, PAGES, REPO, type Page } from './lib/site.ts';
  import { CATALOGS, LOCALES, type Locale, locale, pathOf, t } from './i18n/index.ts';
  import Wander from './lib/Wander.svelte';
  import Suggest from './lib/Suggest.svelte';
  import Home from './pages/Home.svelte';
  import Features from './pages/Features.svelte';
  import Compare from './pages/Compare.svelte';
  import CARDS from './lib/cards.json';

  let { page }: { page: Page } = $props();

  /** Open Graph names a language with its territory: pt_BR, en_US. */
  const OG: Record<string, string> = { en: 'en_US', 'pt-BR': 'pt_BR', es: 'es_ES', 'zh-CN': 'zh_CN', hi: 'hi_IN', fr: 'fr_FR', de: 'de_DE', ja: 'ja_JP', ru: 'ru_RU', id: 'id_ID', ko: 'ko_KR' };
  /** Each page's title and description, for the tab, search engines and link previews, and the headline its preview's
   * card shows (its image's text alternative). */
  const HEAD: Record<Page, { title: string; description: string; card: string }> = {
    home: { title: t('page.meta.title'), description: t('page.meta.description'), card: `${t('page.hero.title')} ${t('page.hero.accent')}` },
    features: { title: t('features.meta.title'), description: t('features.meta.description'), card: t('features.card') },
    compare: { title: t('compare.meta.title'), description: t('compare.meta.description'), card: t('compare.meta.title') },
  };
  const head = $derived(HEAD[page]);
  /** Its link preview's card, drawn in its language by scripts/cards.mjs. */
  const card = $derived(`${ORIGIN}/og/${locale()}/${CARDS[locale()][page]}`);
  /** For search engines, on the first page: the site's name and who makes it, as structured data (a `<` escaped, so
   * nothing in it can close its script element). */
  const site = JSON.stringify([
    { '@context': 'https://schema.org', '@type': 'WebSite', name: NAME, url: `${ORIGIN}/` },
    { '@context': 'https://schema.org', '@type': 'Organization', name: NAME, url: `${ORIGIN}/`, logo: `${ORIGIN}/apple-touch-icon.png`, sameAs: [REPO] },
  ]).replaceAll('<', '\\u003c');
  /** This page's address in `other`. */
  const at = (other: Locale) => `${ORIGIN}${pathOf(other, PAGES[page])}`;
</script>

<!-- The page in its language, and every other language's address: one page per language, each pointing to all. -->
<svelte:head>
  <title>{head.title}</title>
  <meta name="description" content={head.description} />
  <link rel="canonical" href={at(locale())} />
  {#each LOCALES as other (other)}<link rel="alternate" hreflang={other} href={at(other)} />{/each}
  <link rel="alternate" hreflang="x-default" href={at('en')} />
  <meta property="og:type" content="website" />
  <meta property="og:site_name" content={NAME} />
  <meta property="og:title" content={head.title} />
  <meta property="og:description" content={head.description} />
  <meta property="og:url" content={at(locale())} />
  <meta property="og:image" content={card} />
  <meta property="og:image:type" content="image/png" />
  <meta property="og:image:width" content="1200" />
  <meta property="og:image:height" content="630" />
  <meta property="og:image:alt" content={head.card} />
  <meta name="twitter:card" content="summary_large_image" />
  <meta property="og:locale" content={OG[locale()]} />
  {#each LOCALES.filter((other) => other !== locale()) as other (other)}<meta property="og:locale:alternate" content={OG[other]} />{/each}
  {#if page === 'home'}{@html `<script type="application/ld+json">${site}</script>`}{/if}
</svelte:head>

<a href="#main" class="sr-only focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-50 focus:rounded-lg focus:bg-surface focus:px-3 focus:py-2 focus:shadow-lg"
  >{t('page.skip')}</a
>

<Wander />
<Nav {page} />
<Suggest current={locale()} catalogs={CATALOGS} area="page" page={PAGES[page]} />

<main id="main">
  {#if page === 'features'}
    <Features />
  {:else if page === 'compare'}
    <Compare />
  {:else}
    <Home />
  {/if}
</main>

<!-- The last word: the name written large, in its own colors. Decoration only, so no footer landmark: it would be
     empty to assistive technology. -->
<div class="relative overflow-hidden bg-linear-to-b from-paper via-wash to-accent-soft">
  <p
    class="pointer-events-none select-none whitespace-nowrap pb-[2vw] pt-20 text-center font-display text-[clamp(5rem,23vw,24rem)] font-bold leading-[0.8] tracking-[-0.06em] text-ink"
    aria-hidden="true"
  >
    easi<span class="text-you">.</span>run
  </p>
</div>
