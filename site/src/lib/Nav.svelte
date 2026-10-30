<script lang="ts">
  // The menu: a floating bar with the mark, the pages as icon-and-label buttons (the one shown tinted blue and marked
  // aria-current), the documentation, the license and the repository. A phone shows the pages as their icons alone, the
  // demo's aside: the mark leads to it.
  import Wordmark from './Wordmark.svelte';
  import GitHubButton from './GitHubButton.svelte';
  import { NAME, PAGES, type Page } from './site.ts';
  import { focus } from './focus.ts';
  import { locale, pathOf, t } from '../i18n/index.ts';
  import Languages from './Languages.svelte';

  let { page }: { page: Page } = $props();

  // Each page with its icon (24-unit strokes): a play button for the live demo on the first page, four tiles for the
  // features, two columns for the comparison; and the documentation, on its own site.
  const links: { page?: Page; label: string; icon: string; href: string }[] = [
    { page: 'home', label: t('page.nav.demo'), icon: 'M8 5.5v13l10.5-6.5z', href: `${pathOf(locale())}#demo` },
    { page: 'features', label: t('page.nav.features'), icon: 'M4 4h6.5v6.5H4zM13.5 4H20v6.5h-6.5zM4 13.5h6.5V20H4zM13.5 13.5H20V20h-6.5z', href: pathOf(locale(), PAGES.features) },
    { page: 'compare', label: t('page.nav.compare'), icon: 'M4 5h7v14H4zM13 5h7v14h-7zM7 9h1M7 13h1M16 9h1M16 13h1', href: pathOf(locale(), PAGES.compare) },
    { label: t('page.nav.docs'), icon: 'M5 4h10l4 4v12H5zM15 4v4h4M8 12h8M8 16h6', href: `https://docs.easi.run${pathOf(locale())}` },
  ];
</script>

<header class="sticky top-3 z-40 px-4">
  <nav
    class="mx-auto flex h-16 max-w-[66rem] items-center justify-between gap-2 rounded-2xl border border-line bg-surface/90 pl-4 pr-2 sm:gap-4 sm:pl-5 sm:pr-3 shadow-[0_10px_34px_-14px_rgb(20_40_75/0.28)] backdrop-blur-md"
    aria-label={t('page.nav.label')}
  >
    <a href={page === 'home' ? '#top' : pathOf(locale())} class="flex items-center rounded-lg {focus}" aria-label={NAME}>
      <Wordmark class="text-[20px] sm:text-[22px]" mark />
    </a>

    <ul class="flex items-center gap-0.5 sm:gap-1">
      {#each links as link (link.href)}
        {@const on = link.page === page}
        <li class={link.page === 'home' ? 'max-md:hidden' : undefined}>
          <a
            href={link.href}
            class="flex items-center gap-2 rounded-xl px-2 py-2 text-[15px] font-semibold sm:px-2.5 md:px-3.5 transition {on ? 'bg-accent-soft text-accent' : 'text-ink/75 hover:bg-wash hover:text-ink'} {focus}"
            aria-current={on ? 'page' : undefined}
          >
            <svg viewBox="0 0 24 24" class="size-[18px] {on ? 'text-accent' : 'text-accent/70'}" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
              ><path d={link.icon} /></svg
            ><span class="max-md:sr-only">{link.label}</span>
          </a>
        </li>
      {/each}
    </ul>

    <div class="flex items-center gap-1.5 sm:gap-3">
      <Languages current={locale()} label={t('page.nav.language')} page={PAGES[page]} />
      <span class="hidden text-sm font-semibold text-muted lg:inline">AGPL-3.0</span>
      <GitHubButton class="rounded-xl px-3 py-2 text-[15px]" />
    </div>
  </nav>
</header>

