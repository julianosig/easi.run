<script lang="ts">
  import Compare from './lib/Compare.svelte';
  import { TECH } from './lib/tech.ts';
  import Nav from './lib/Nav.svelte';
  import SectionHeader from './lib/SectionHeader.svelte';
  import { ORIGIN, REPO } from './lib/site.ts';
  import { CATALOGS, LOCALES, locale, pathOf, t } from './i18n/index.ts';
  import LiveDemo from './lib/LiveDemo.svelte';
  import Wander from './lib/Wander.svelte';
  import Suggest from './lib/Suggest.svelte';
  import Takeover from './lib/Takeover.svelte';
  import Vault from './lib/Vault.svelte';
  import { focus } from './lib/focus.ts';

  const doc = (path: string) => `${REPO}/blob/main/docs/${path}`;

  const wrap = 'mx-auto max-w-[70rem] px-6';
  /** Open Graph names a language with its territory: pt_BR, en_US. */
  const OG: Record<string, string> = { en: 'en_US', 'pt-BR': 'pt_BR', es: 'es_ES', 'zh-CN': 'zh_CN', hi: 'hi_IN', fr: 'fr_FR', de: 'de_DE', ja: 'ja_JP', ru: 'ru_RU', id: 'id_ID', ko: 'ko_KR' };
  /** When the comparison was checked, as the page's language writes a month. */
  const checked = new Intl.DateTimeFormat(locale(), { month: 'short', year: 'numeric', timeZone: 'UTC' }).format(Date.UTC(2026, 9));
</script>

<!-- The page in its language, and every other language's address: one page per language, each pointing to all. -->
<svelte:head>
  <meta name="description" content={t('page.meta.description')} />
  <link rel="canonical" href={`${ORIGIN}${pathOf(locale())}`} />
  {#each LOCALES as other (other)}<link rel="alternate" hreflang={other} href={`${ORIGIN}${pathOf(other)}`} />{/each}
  <link rel="alternate" hreflang="x-default" href={`${ORIGIN}/`} />
  <meta property="og:type" content="website" />
  <meta property="og:title" content="easi.run" />
  <meta property="og:description" content={t('page.meta.description')} />
  <meta property="og:url" content={`${ORIGIN}${pathOf(locale())}`} />
  <meta property="og:locale" content={OG[locale()]} />
  {#each LOCALES.filter((other) => other !== locale()) as other (other)}<meta property="og:locale:alternate" content={OG[other]} />{/each}
</svelte:head>

<a href="#main" class="sr-only focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-50 focus:rounded-lg focus:bg-surface focus:px-3 focus:py-2 focus:shadow-lg"
  >{t('page.skip')}</a
>

<Wander />
<Nav />
<Suggest current={locale()} catalogs={CATALOGS} area="page" />

<main id="main">
  <!-- Hero -->
  <section id="demo" class="relative overflow-x-clip">
    <div class="pointer-events-none absolute left-1/2 top-[26rem] -z-10 h-[44rem] w-[90rem] -translate-x-1/2 bg-[radial-gradient(closest-side,rgb(36_87_245/0.14),rgb(120_160_255/0.07)_55%,transparent)]" aria-hidden="true"></div>
    <div class="{wrap} pt-20 text-center">
      <h1 class="mx-auto max-w-4xl text-[clamp(2rem,3.6vw,3.25rem)] font-semibold leading-[1.08] tracking-[-0.03em]">
        {t('page.hero.title')} <span class="block text-accent">{t('page.hero.accent')}</span>
      </h1>
      <p class="mx-auto mt-5 max-w-5xl text-pretty text-lg leading-relaxed text-muted">{t('page.hero.lead')}</p>
    </div>

    <div class={wrap}>
      <LiveDemo />
    </div>

    <!-- The two who use it, side by side: the same browser, each for their own need. -->
    <div class="{wrap} mt-14">
      <ul class="mx-auto grid max-w-4xl gap-4 sm:grid-cols-2" aria-label={t('page.for.label')}>
        <li class="rounded-2xl bg-wash p-6">
          <Takeover />
          <h2 class="mt-5 text-lg font-semibold tracking-tight">{t('page.for.agents.title')}</h2>
          <p class="mt-2 text-[15px] leading-relaxed text-muted">{t('page.for.agents.text')}</p>
        </li>
        <li class="rounded-2xl bg-wash p-6">
          <Vault />
          <h2 class="mt-5 text-lg font-semibold tracking-tight">{t('page.for.you.title')}</h2>
          <p class="mt-2 text-[15px] leading-relaxed text-muted">{t('page.for.you.text')}</p>
        </li>
      </ul>
    </div>

    <!-- What I am built on, under the preview. -->
    <div class="{wrap} mt-14 text-center">
      <ul class="flex flex-wrap items-center justify-center gap-x-6 gap-y-3" aria-label={t('page.builtOn')}>
        {#each TECH as tech (tech.name)}
          <li class="flex items-center gap-2 text-sm text-muted">
            <svg viewBox="0 0 24 24" class="size-5 fill-current" aria-hidden="true"><path d={tech.d} /></svg>{tech.name}
          </li>
        {/each}
      </ul>
    </div>
  </section>

  <!-- Compare -->
  <section id="compare" class="{wrap} py-24">
    <SectionHeader>{t('compare.title')}</SectionHeader>
    <p class="mx-auto mt-4 max-w-4xl text-center text-base leading-relaxed text-muted">{t('compare.intro')}</p>
    <div class="mt-10"><Compare /></div>
    <p class="mt-5 text-center text-xs text-muted">
      {t('compare.legend', { date: checked })}
      <a href={doc('comparison.md')} class="font-medium text-accent hover:underline {focus}">{t('compare.legend.sources')}</a>
      {t('compare.legend.owners')}
    </p>
  </section>

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
