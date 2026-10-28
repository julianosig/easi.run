<script lang="ts">
  // The logged-in admin's frame: a sidebar with the mark, one link per page (the open one marked) and Log out at its
  // foot, its language above it, and the page beside it. On a narrow screen the sidebar is a rail of icons; each link keeps its name.
  import type { Snippet } from 'svelte';
  import { t } from '../i18n/index.svelte.ts';
  import { LOG_OUT_ICON, PAGE_ICONS } from './admin.ts';
  import Icon from './Icon.svelte';
  import LanguagePicker from './LanguagePicker.svelte';
  import Mark from './Mark.svelte';
  import { hashOf, PAGES, type Page } from './route.ts';
  import SourceNote from './SourceNote.svelte';
  import { focusRing } from './ui.ts';
  import Wordmark from './Wordmark.svelte';

  let { page, onLogout, children }: { page: Page; onLogout: () => void; children: Snippet } = $props();
  const item = `flex items-center justify-center gap-3 rounded-xl p-2.5 text-[15px] font-semibold transition md:justify-start md:px-3 ${focusRing}`;
</script>

<div class="min-h-dvh bg-paper">
  <aside class="fixed inset-y-0 left-0 z-30 flex w-16 flex-col bg-wash px-2 py-4 md:w-60 md:px-4 md:py-6">
    <a class="mb-6 flex justify-center rounded-lg md:mb-8 md:justify-start md:px-2 {focusRing}" href={hashOf({ page: 'sessions' })}>
      <Mark class="h-7 w-auto md:hidden" /><Wordmark class="hidden text-[22px] md:inline" /><span class="sr-only md:hidden">easi.run</span>
    </a>
    <nav aria-label={t('shell.nav')}>
      <ul class="space-y-1">
        {#each PAGES as p (p)}
          <li>
            <a
              class="{item} {p === page ? 'bg-paper text-ink shadow-[0_6px_18px_-10px_rgb(20_40_75/0.35)] [&_svg]:text-accent' : 'text-ink/70 hover:bg-paper/70 hover:text-ink [&_svg]:text-ink/50'}"
              href={hashOf({ page: p })}
              aria-current={p === page ? 'page' : undefined}
              title={t(`shell.page.${p}`)}
              ><Icon d={PAGE_ICONS[p]} class="size-5 shrink-0" /><span class="sr-only md:not-sr-only">{t(`shell.page.${p}`)}</span></a
            >
          </li>
        {/each}
      </ul>
    </nav>
    <div class="mt-auto space-y-1">
      <LanguagePicker />
      <button class="{item} w-full text-ink/70 hover:bg-paper/70 hover:text-ink [&_svg]:text-ink/50" type="button" title={t('shell.logout')} onclick={onLogout}
        ><Icon d={LOG_OUT_ICON} class="size-5 shrink-0" /><span class="sr-only md:not-sr-only">{t('shell.logout')}</span></button
      >
    </div>
  </aside>
  <main class="pl-16 md:pl-60">
    <div class="mx-auto max-w-4xl px-4 pb-10 pt-8 sm:px-8 md:pt-12 lg:px-12">
      {@render children()}
      <SourceNote class="mt-20" />
    </div>
  </main>
</div>
