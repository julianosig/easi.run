<script lang="ts">
  // The replays the admin keeps (`GET /api/runs`): what happened in each session that stopped, its screen and its
  // events, told together by session (a saved session's runs under its name, newest first), and narrowed to the days
  // asked for. Each opens in the player, as a running session's does on its page, and can be forgotten now. After a
  // restart only the password opens them.
  import { followLists } from './updates.ts';
  import Button from './Button.svelte';
  import EmptyState from './EmptyState.svelte';
  import Icon from './Icon.svelte';
  import PageHeader from './PageHeader.svelte';
  import Replay from './Replay.svelte';
  import Skeleton from './Skeleton.svelte';
  import Unlock from './Unlock.svelte';
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { chip, CHIP_TONES, row, rowBadge, rows } from './admin.ts';
  import { api, type Run, statusOf } from './api.ts';
  import { errorOf } from './errors.ts';
  import { dayOf, groupsOf, within } from './replays.ts';
  import { focusRing } from './ui.ts';

  let { onLogin }: { onLogin: () => void } = $props();

  /** Null until they are read. */
  let runs: Run[] | null = $state(null);
  /** How many archives only the password opens. */
  let locked = $state(0);
  let unlocking = $state(false);
  /** The run whose timeline and replay are open over the page. */
  let open: Run | null = $state(null);
  let failure: Said | null = $state(null);
  /** The days asked for, each as a date field gives it (`2026-11-18`), or empty for no bound. */
  let from = $state('');
  let to = $state('');

  // The latest load's answer only: one asked before a forget (or another load) changes nothing once it comes.
  let turn = 0;
  async function load() {
    const mine = ++turn;
    try {
      const listed = await api<{ runs: Run[]; locked: number }>('GET', '/api/runs');
      if (mine !== turn) return;
      [runs, locked, failure] = [listed.runs, listed.locked, null];
    } catch (e) {
      if (mine !== turn) return;
      if (statusOf(e) === 401) return onLogin();
      failure = errorOf(e);
    }
  }
  // Again when it changes: a session that stops meanwhile joins the list.
  $effect(() => {
    void load();
    return followLists(() => void load());
  });

  async function forget(run: Run) {
    if (!confirm(t('timeline.runs.forget.confirm', { name: run.label }))) return;
    try {
      await api('DELETE', `/api/runs/${run.id}`);
      if (open?.id === run.id) open = null;
      await load();
    } catch (e) {
      if (statusOf(e) === 401) return onLogin();
      failure = errorOf(e);
    }
  }

  /** The first and the last day any replay holds: the dates can narrow the list only when they differ. */
  const span = $derived.by(() => {
    if (!runs?.length) return null;
    const first = dayOf(Math.min(...runs.map((r) => r.started)));
    const last = dayOf(Math.max(...runs.map((r) => r.stopped)));
    return first === last ? null : { first, last };
  });
  // Dates that no longer narrow anything (the list shrank to one day) bound nothing, and are forgotten with their
  // fields: they do not come back to bound a list that grew again.
  const bounds = $derived.by(() => (span ? { from, to } : { from: '', to: '' }));
  $effect(() => {
    if (!span && runs !== null && (from || to)) [from, to] = ['', ''];
  });
  // Told together first, by every run kept (a run names its saved session, or is called as one that does), then
  // narrowed to the dates: the runs of a group that are in them, the group whose newest of those is newest first.
  const groups = $derived.by(() =>
    groupsOf(runs ?? [])
      .map((group) => ({ ...group, runs: group.runs.filter((r) => within(r, bounds.from, bounds.to)) }))
      .filter((group) => group.runs.length > 0)
      .sort((a, b) => b.runs[0]!.stopped - a.runs[0]!.stopped),
  );

  /** A day and a time of it, with its year (`true`) for a run of another year than this one. */
  const ranges = $derived.by(() => {
    const of = (year: boolean) => new Intl.DateTimeFormat(i18n.locale, { year: year ? 'numeric' : undefined, month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
    return { thisYear: of(false), another: of(true) };
  });
  /** When a run went on: its start to its end, the day said once when both are in it. */
  const when = (run: Run) => {
    const same = new Date(run.started).getFullYear() === new Date().getFullYear();
    return (same ? ranges.thisYear : ranges.another).formatRange(run.started, Math.max(run.started, run.stopped));
  };
  const dateField = `min-w-0 flex-1 rounded-xl border border-line bg-paper px-3 py-2 text-sm text-ink focus:border-accent focus:outline-3 focus:outline-accent-soft sm:flex-none`;
</script>

<PageHeader icon={ICONS.replay} title={t('shell.page.replays')} subtitle={t('timeline.runs.subtitle')} />

<p class="mt-6 font-semibold text-bad empty:hidden" role="alert">{failure?.()}</p>

{#if locked > 0}
  <div class="mt-8 flex flex-wrap items-center justify-between gap-4 rounded-2xl bg-wash px-5 py-4">
    <p class="max-w-lg text-sm text-muted">{t('timeline.runs.locked')}</p>
    <Button variant="primary" icon={ICONS.unlock} onclick={() => (unlocking = true)}>{t('sessions.unlock')}</Button>
  </div>
{/if}

{#if runs === null}
  {#if !failure}<Skeleton said={t('sessions.loading')} />{/if}
{:else if runs.length === 0}
  {#if locked === 0}<EmptyState points={[t('timeline.replays.empty')]} />{/if}
{:else}
  {#if span}
    <!-- The dates show only while the replays are of more than one day: with one, they would narrow nothing. -->
    <div class="mt-10 flex flex-wrap items-center gap-2" role="group" aria-label={t('timeline.replays.dates')}>
      <span class="flex min-w-0 flex-1 items-center gap-2 sm:flex-none">
      <input class={dateField} type="date" aria-label={t('timeline.replays.from')} min={span.first} max={to || span.last} bind:value={from} />
      <span class="text-muted" aria-hidden="true">–</span>
      <input class={dateField} type="date" aria-label={t('timeline.replays.to')} min={from || span.first} max={span.last} bind:value={to} />
      </span>
      {#if from || to}
        <button class="rounded-full px-3.5 py-1.5 text-sm font-semibold text-muted transition hover:bg-wash hover:text-ink {focusRing}" type="button" onclick={() => ((from = ''), (to = ''))}>{t('timeline.replays.all')}</button>
      {/if}
    </div>
  {/if}
  {#if groups.length === 0}
    <p class="mt-6 text-sm text-muted">{t('timeline.replays.none')}</p>
  {/if}
  <div class={span ? 'mt-2' : 'mt-6'}>
    {#each groups as group (group.key)}
      <section class="mt-6" aria-label={group.name}>
        <h2 class="flex min-w-0 items-center gap-3">
          <span class={rowBadge}><Icon d={group.saved ? ICONS.persistent : ICONS.website} class="size-5" /></span>
          <span class="truncate font-semibold">{group.name}</span>
          {#if group.runs.length > 1}<span class="{chip} {CHIP_TONES.neutral} tabular-nums">{group.runs.length}</span>{/if}
        </h2>
        <!-- Its runs, under its name: each where the name starts. -->
        <ul class="mt-1 pl-[3.25rem] {rows}">
          {#each group.runs as run (run.id)}
            <!-- On a narrow screen its time takes a line of its own, its actions the next. -->
            <li class="flex flex-wrap items-center gap-x-4 gap-y-1.5 py-2 {row}">
              <span class="w-full text-sm tabular-nums text-muted sm:w-auto sm:min-w-0 sm:flex-1">{when(run)}</span>
              <Button size="sm" icon={ICONS.timeline} onclick={() => (open = run)}>{t('timeline.runs.open')}</Button>
              <Button variant="ghost" tone="danger" size="sm" icon={ICONS.delete} label={t('timeline.runs.forget')} onclick={() => forget(run)} />
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  </div>
{/if}

<Unlock bind:open={unlocking} subtitle={t('timeline.runs.locked')} onUnlocked={() => void load()} {onLogin} />

{#if open}
  {#key open.id}<Replay id={open.id} label={open.label} live={false} at={null} onClose={() => (open = null)} {onLogin} />{/key}
{/if}
