<script lang="ts">
  // The menu: a floating bar with the mark, the sections as icon-and-label buttons (the one on screen tinted blue and
  // marked aria-current), the documentation, the license and the repository.
  import Wordmark from './Wordmark.svelte';
  import GitHubButton from './GitHubButton.svelte';
  import { NAME } from './site.ts';
  import { focus } from './focus.ts';
  import { t } from '../i18n/index.ts';
  import Languages from './Languages.svelte';

  // Each section with its icon (24-unit strokes): a play button for the live demo in the hero, two columns for the
  // comparison; and the documentation, on its own site.
  const links: { id: string; label: string; icon: string; href?: string }[] = [
    { id: 'demo', label: t('page.nav.demo'), icon: 'M8 5.5v13l10.5-6.5z' },
    { id: 'compare', label: t('page.nav.compare'), icon: 'M4 5h7v14H4zM13 5h7v14h-7zM7 9h1M7 13h1M16 9h1M16 13h1' },
    { id: 'docs', label: t('page.nav.docs'), icon: 'M5 4h10l4 4v12H5zM15 4v4h4M8 12h8M8 16h6', href: 'https://docs.easi.run/' },
  ];
  let current = $state('');
  let height = $state(0);

  // A section counts as current while it crosses the middle band of the viewport. The band is set in pixels from the
  // viewport height (percentages in rootMargin would follow its width), so it is rebuilt when the height changes.
  $effect(() => {
    if (!height) return;
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) if (entry.isIntersecting) current = entry.target.id;
      },
      { rootMargin: `-${Math.round(height * 0.45)}px 0px -${Math.round(height * 0.5)}px 0px` },
    );
    for (const { id } of links) {
      const section = document.getElementById(id);
      if (section) observer.observe(section);
    }
    return () => observer.disconnect();
  });
</script>

<svelte:window bind:innerHeight={height} />

<header class="sticky top-3 z-40 px-4">
  <nav
    class="mx-auto flex h-16 max-w-[66rem] items-center justify-between gap-4 rounded-2xl border border-line bg-surface/90 pl-5 pr-3 shadow-[0_10px_34px_-14px_rgb(20_40_75/0.28)] backdrop-blur-md"
    aria-label={t('page.nav.label')}
  >
    <a href="#top" class="flex items-center rounded-lg {focus}" aria-label={NAME}>
      <Wordmark class="text-[22px]" mark />
    </a>

    <ul class="hidden items-center gap-1 md:flex">
      {#each links as link (link.id)}
        {@const on = current === link.id}
        <li>
          <a
            href={link.href ?? `#${link.id}`}
            class="flex items-center gap-2 rounded-xl px-3.5 py-2 text-[15px] font-semibold transition {on ? 'bg-accent-soft text-accent' : 'text-ink/75 hover:bg-wash hover:text-ink'} {focus}"
            aria-current={on ? 'location' : undefined}
          >
            <svg viewBox="0 0 24 24" class="size-[18px] {on ? 'text-accent' : 'text-accent/70'}" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
              ><path d={link.icon} /></svg
            >{link.label}
          </a>
        </li>
      {/each}
    </ul>

    <div class="flex items-center gap-3">
      <Languages />
      <span class="hidden text-sm font-semibold text-muted lg:inline">AGPL-3.0</span>
      <GitHubButton class="rounded-xl px-3 py-2 text-[15px]" />
    </div>
  </nav>
</header>

