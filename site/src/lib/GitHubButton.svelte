<script module lang="ts">
  import { REPO } from './site.ts';
  import { focus } from './focus.ts';

  // One request per page for the star count, shared by every button; no answer means no count.
  let request: Promise<number | undefined> | undefined;
  function starCount() {
    request ??= fetch(REPO.replace('https://github.com/', 'https://api.github.com/repos/'))
      .then((r) => (r.ok ? r.json() : undefined))
      .then((repo) => (typeof repo?.stargazers_count === 'number' ? repo.stargazers_count : undefined))
      .catch(() => undefined);
    return request;
  }
</script>

<script lang="ts">
  import { t } from '../i18n/index.ts';
  // A link to the repository with GitHub's mark and, once GitHub answers, its star count.

  let { class: className = '' }: { class?: string } = $props();

  let stars: number | undefined = $state();
  $effect(() => {
    let live = true;
    starCount().then((n) => {
      if (live) stars = n;
    });
    return () => {
      live = false;
    };
  });
  const compact = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(n >= 10000 ? 0 : 1)}k` : String(n));
</script>

<a
  href={REPO}
  class="inline-flex items-center gap-2 whitespace-nowrap font-semibold transition {focus} bg-surface text-ink ring-1 ring-line hover:ring-ink/30 {className}"
>
  <svg viewBox="0 0 16 16" class="size-[18px]" fill="currentColor" aria-hidden="true"
    ><path
      d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z"
    /></svg
  >
  <!-- On a phone, the mark alone says it, beside the menu's pages; the count and the word come back where there is room. -->
  <span class="max-sm:sr-only">{t('page.github.star')}</span>
  {#if stars !== undefined}
    <span class="flex items-center gap-1 border-l border-line pl-2 max-sm:hidden"
      ><svg viewBox="0 0 16 16" class="size-3.5 text-[#e3a008]" fill="currentColor" aria-hidden="true"><path d="M8 .9l2.2 4.5 4.9.7-3.6 3.5.9 4.9L8 12.2l-4.4 2.3.9-4.9L.9 6.1l4.9-.7z" /></svg>{compact(stars)}<span
        class="sr-only">{t('page.github.stars')}</span
      ></span
    >
  {/if}
</a>
