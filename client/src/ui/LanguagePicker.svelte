<script lang="ts">
  // The page's language: its browser's, or one the person picks, each named in itself. A native select, so every
  // keyboard and screen reader knows it; on the sidebar's narrow rail only its globe shows, the select over it.
  import { i18n, setLocale, t } from '../i18n/index.svelte.ts';
  import { AUTONYMS, isLocale, LOCALES } from '../i18n/locales.ts';
  import Icon from './Icon.svelte';
  import { focusRing } from './ui.ts';

  let { class: className = '' }: { class?: string } = $props();
  const GLOBE = 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM3.6 9h16.8M3.6 15h16.8M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18';
</script>

<label class="relative flex items-center justify-center gap-3 rounded-xl p-2.5 text-[15px] font-semibold text-ink/70 transition focus-within:bg-paper/70 hover:bg-paper/70 hover:text-ink md:justify-start md:px-3 {className}" title={t('shell.language')}>
  <Icon d={GLOBE} class="size-5 shrink-0 text-ink/50" />
  <span class="sr-only">{t('shell.language')}</span>
  <span class="sr-only truncate md:not-sr-only" aria-hidden="true">{AUTONYMS[i18n.locale]}</span>
  <select
    class="absolute inset-0 cursor-pointer appearance-none opacity-0 {focusRing}"
    value={i18n.chosen ?? ''}
    onchange={(e) => {
      const value = e.currentTarget.value;
      void setLocale(isLocale(value) ? value : null);
    }}
  >
    <option value="">{t('shell.language.browser')}</option>
    {#each LOCALES as locale (locale)}<option value={locale} lang={locale}>{AUTONYMS[locale]}</option>{/each}
  </select>
</label>
