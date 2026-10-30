<script lang="ts">
  // The comparison's page: what hurts, then this product head to head with each rival, and where the facts come from.
  import Table from '../lib/Compare.svelte';
  import SectionHeader from '../lib/SectionHeader.svelte';
  import { REPO } from '../lib/site.ts';
  import { focus } from '../lib/focus.ts';
  import { locale, t } from '../i18n/index.ts';

  /** When the comparison was checked, as the page's language writes a month. */
  const checked = new Intl.DateTimeFormat(locale(), { month: 'short', year: 'numeric', timeZone: 'UTC' }).format(Date.UTC(2026, 9));
</script>

<section class="mx-auto max-w-[70rem] px-6 pb-24 pt-20">
  <SectionHeader tag="h1">{t('compare.title')}</SectionHeader>
  <p class="mx-auto mt-4 max-w-4xl text-center text-base leading-relaxed text-muted">{t('compare.intro')}</p>
  <div class="mt-10"><Table /></div>
  <p class="mt-5 text-center text-xs text-muted">
    {t('compare.legend', { date: checked })}
    <a href={`${REPO}/blob/main/docs/comparison.md`} class="font-medium text-accent hover:underline {focus}">{t('compare.legend.sources')}</a>
    {t('compare.legend.owners')}
  </p>
</section>
