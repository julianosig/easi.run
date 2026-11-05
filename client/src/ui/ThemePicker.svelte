<script lang="ts">
  // The page's look: the system's, or one the person picks. A native select, as the language's beside it (LanguagePicker).
  import { chooseTheme, chosenTheme, THEMES, type Theme } from '../../../i18n/theme.ts';
  import { t } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';
  import { ICONS } from './icons.ts';
  import { focusRing } from './ui.ts';

  let theme: Theme = $state(chosenTheme());
</script>

<label class="relative flex items-center justify-center gap-3 rounded-xl p-2.5 text-[15px] font-semibold text-ink/70 transition hover:bg-paper/70 hover:text-ink md:justify-start">
  <Icon d={ICONS.theme} class="size-5 shrink-0 text-ink/50" />
  <span class="sr-only">{t('shell.theme')}</span>
  <span class="sr-only truncate md:not-sr-only" aria-hidden="true">{t(`shell.theme.${theme}`)}</span>
  <select
    class="absolute inset-0 cursor-pointer appearance-none opacity-0 {focusRing}"
    bind:value={theme}
    onchange={() => chooseTheme(theme)}
  >
    {#each THEMES as option (option)}<option value={option}>{t(`shell.theme.${option}`)}</option>{/each}
  </select>
</label>
