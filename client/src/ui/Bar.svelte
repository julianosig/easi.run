<script lang="ts">
  // The demo door's top bar, as the landing's (site/src/lib/Nav.svelte): a floating bar that stays at the top of the
  // page, with the mark and the name.
  import { t } from '../i18n/index.svelte.ts';
  import Wordmark from './Wordmark.svelte';
</script>

<header class="sticky top-3 z-40">
  <nav
    class="mx-auto flex h-16 max-w-[76rem] items-center justify-between gap-2 rounded-2xl border border-line bg-paper/90 pl-5 pr-3 shadow-[0_10px_34px_-14px_rgb(20_40_75/0.28)] backdrop-blur-md sm:gap-4"
    aria-label={t('common.nav')}
  >
    <Wordmark class="text-[22px]" />
  </nav>
</header>
