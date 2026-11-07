<script lang="ts">
  // Above every page of an installation's admin: a newer release than the one it runs is out, and the command that
  // moves it there. The release it runs is the admin's (`/api/me`); the latest, what https://easi.run/release.json names,
  // with the releases before it.
  // Below it, what changed since the release it runs, as https://easi.run/changelog.json tells it in the page's language.
  // Hidden for a release, it stays hidden until a newer one is out.
  import Button from './Button.svelte';
  import { ICONS } from './icons.ts';
  import Icon from './Icon.svelte';
  import { i18n, t } from '../i18n/index.svelte.ts';
  import { copyText, me } from './api.ts';

  const COMMAND = 'curl -fsSLO https://easi.run/install.sh && bash install.sh --update';
  const HIDDEN = 'easi-run-update-hidden';
  const COMMIT = /^[0-9a-f]{40}$/;

  /** The days shown at first; the earlier ones wait for a click. */
  const FIRST = 3;

  type Release = { commit: string; date: string; changes: Record<string, string>[] };

  let latest: string | null = $state(null);
  let copied = $state(false);
  let releases: Release[] = $state([]);
  let all = $state(false);
  // A day's releases read as one.
  const days = $derived(
    releases.reduce<{ date: string; changes: Record<string, string>[] }[]>((list, release) => {
      const last = list.at(-1);
      if (last?.date === release.date) last.changes.push(...release.changes);
      else list.push({ date: release.date, changes: [...release.changes] });
      return list;
    }, []),
  );

  const hidden = (): string | null => {
    try {
      return localStorage.getItem(HIDDEN);
    } catch {
      return null;
    }
  };

  async function check() {
    const running = (await me()).release;
    if (!running) return;
    const answer = await fetch('https://easi.run/release.json', { cache: 'no-store', credentials: 'omit', referrerPolicy: 'no-referrer' });
    if (!answer.ok) return;
    const { commit, earlier } = (await answer.json()) as { commit?: unknown; earlier?: unknown };
    // Newer only when the release it runs is one of the earlier ones (a later one, or one never published, is not).
    const older = Array.isArray(earlier) && earlier.includes(running);
    if (typeof commit === 'string' && COMMIT.test(commit) && older && commit !== hidden()) {
      latest = commit;
      releases = await since([commit, ...(earlier as unknown[])], running);
    }
  }

  /** The changelog's releases newer than `running`, newest first, by their place in `order` (latest first). */
  async function since(order: unknown[], running: string): Promise<Release[]> {
    const answer = await fetch('https://easi.run/changelog.json', { cache: 'no-store', credentials: 'omit', referrerPolicy: 'no-referrer' });
    if (!answer.ok) return [];
    const listed = ((await answer.json()) as { releases?: unknown }).releases;
    if (!Array.isArray(listed)) return [];
    const before = order.indexOf(running);
    return listed.filter((r: Release): r is Release => {
      const place = order.indexOf(r?.commit);
      const text = Array.isArray(r?.changes) && r.changes.every((c) => typeof c?.en === 'string');
      return place >= 0 && place < before && typeof r.date === 'string' && text;
    });
  }

  const said = (change: Record<string, string>): string => {
    const text = change[i18n.locale];
    return typeof text === 'string' ? text : (change.en ?? '');
  };
  const day = (date: string): string => {
    const at = new Date(`${date}T00:00:00Z`);
    return Number.isNaN(at.getTime()) ? date : new Intl.DateTimeFormat(i18n.locale, { dateStyle: 'medium', timeZone: 'UTC' }).format(at);
  };
  // Not reached (offline, say): nothing to say.
  check().catch(() => undefined);

  function hide() {
    try {
      if (latest) localStorage.setItem(HIDDEN, latest);
    } catch {
      // Hidden for this page only.
    }
    latest = null;
    releases = [];
  }
</script>

{#if latest}
  <section class="mb-8 rounded-2xl border border-line bg-wash p-4 sm:p-5" aria-labelledby="update-title">
    <h2 id="update-title" class="flex items-center gap-2 text-[15px] font-semibold"><Icon d={ICONS.retry} class="size-5 shrink-0 text-accent" />{t('shell.update.title')}</h2>
    {#if days.length}
      <h3 class="mt-3 text-sm font-semibold">{t('shell.update.changes')}</h3>
      {#each all ? days : days.slice(0, FIRST) as group (group.date)}
        <div class="mt-2">
          <p class="text-xs text-muted"><time datetime={group.date}>{day(group.date)}</time></p>
          <ul class="mt-1 list-disc space-y-1 pl-5 text-sm">
            {#each group.changes as change, i (i)}<li>{said(change)}</li>{/each}
          </ul>
        </div>
      {/each}
      {#if !all && days.length > FIRST}
        <Button size="sm" variant="ghost" class="mt-2" onclick={() => (all = true)}>{t('shell.update.earlier')}</Button>
      {/if}
    {/if}
    <p class="{days.length ? 'mt-4' : 'mt-1'} text-sm text-muted">{t('shell.update.text')}</p>
    <code class="mt-3 block overflow-x-auto whitespace-nowrap rounded-lg bg-paper px-3 py-2 text-[13px]">{COMMAND}</code>
    <div class="mt-3 flex flex-wrap gap-2">
      <Button size="sm" icon={copied ? ICONS.check : ICONS.copy} onclick={async () => (copied = await copyText(COMMAND))}>{copied ? t('shell.update.copied') : t('shell.update.copy')}</Button>
      <Button size="sm" variant="ghost" icon={ICONS.close} onclick={hide}>{t('shell.update.hide')}</Button>
    </div>
  </section>
{/if}
