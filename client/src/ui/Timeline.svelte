<script lang="ts" module>
  // Where a browser keeps the last event of each session it showed: what came after is new.
  const SEEN = 'easi-run-timeline-seen:';
  export function seenOf(id: string): number {
    try {
      return Number(localStorage.getItem(SEEN + id)) || 0;
    } catch {
      return 0;
    }
  }
  function markSeen(id: string, seq: number) {
    try {
      if (seq > seenOf(id)) localStorage.setItem(SEEN + id, String(seq));
    } catch {
      // Remembered for this page only.
    }
  }
</script>

<script lang="ts">
  // What happened in a session (see timeline.ts): a summary of what is new since this browser last showed it, then every
  // event, newest last, filtered by who it was about or what went wrong, and downloadable as Markdown or JSON. A running
  // session's comes in as it happens.
  import Button from './Button.svelte';
  import Icon from './Icon.svelte';
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { api, statusOf, type TimelineEvent, type TimelinePage } from './api.ts';
  import { errorOf } from './errors.ts';
  import { field } from './ui.ts';
  import { untrack } from 'svelte';
  import { type Filter, leftAt, lines, markdown, merged, shown, summary } from './timeline.ts';

  let {
    id,
    live,
    onLogin,
  }: {
    id: string;
    /** A running session's: it polls for what comes next. */
    live: boolean;
    onLogin: () => void;
  } = $props();

  const POLL_MS = 2000;
  let events: TimelineEvent[] = $state([]);
  let next = 0;
  let first = $state(1);
  let loaded = $state(false);
  let failure: Said | null = $state(null);
  let filter: Filter = $state('all');
  // What the admin calls the session, scrubbed as its events are (none when the scrubbing hid it): the export's title.
  let label = $state('');
  // What was new when it opened: kept as the summary while it stays open (a new id is a new timeline, see its callers).
  const seen = untrack(() => seenOf(id));

  const say = (key: string, params?: Record<string, string | number>) => t(`timeline.${key}` as Parameters<typeof t>[0], params as never);
  const time = (ms: number) => new Date(ms).toLocaleTimeString(i18n.locale);
  const said = $derived(lines(events, say));
  const fresh = $derived(events.filter((e) => e.seq > seen));
  const news = $derived(summary(fresh));
  const left = $derived(leftAt(events));

  // One read at a time: a poll while one is out waits for the next tick.
  let reading = false;
  async function read() {
    if (reading) return;
    reading = true;
    try {
      const page = await api<TimelinePage>('GET', `/api/sessions/${id}/timeline?after=${next}`);
      events = merged(events, page.events, 2000);
      next = Math.max(next, page.next - 1);
      first = page.first;
      if (typeof page.label === 'string') label = page.label;
      loaded = true;
      failure = null;
      markSeen(id, next);
    } catch (e) {
      if (statusOf(e) === 401) return onLogin();
      failure = errorOf(e);
    } finally {
      reading = false;
    }
  }

  $effect(() => {
    void read();
    if (!live) return;
    const timer = setInterval(read, POLL_MS);
    return () => clearInterval(timer);
  });

  function save(name: string, type: string, body: string) {
    const url = URL.createObjectURL(new Blob([body], { type }));
    const a = Object.assign(document.createElement('a'), { href: url, download: name });
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  const exported = () => `easi-run-${id}-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, '')}`;

  const TONES = { plain: 'text-ink', problem: 'text-bad', waiting: 'text-accent', done: 'text-good' } as const;
  const ACTOR_ICONS = { agent: ICONS.devtools, people: ICONS.watch, page: ICONS.website } as const;
</script>

<div class="flex min-h-0 flex-col">
  <p class="text-sm font-semibold text-bad empty:hidden" role="alert">{failure?.()}</p>
  {#if !loaded && !failure}
    <p class="text-sm text-muted">{t('timeline.loading')}</p>
  {:else if loaded}
    <section class="rounded-2xl bg-wash p-4" aria-labelledby="timeline-new">
      <h3 id="timeline-new" class="text-[15px] font-semibold">
        {left ? t('timeline.since.left', { time: time(left) }) : t('timeline.since.looked')}
      </h3>
      {#if fresh.length === 0}
        <p class="mt-1 text-sm text-muted">{t('timeline.since.nothing')}</p>
      {:else}
        <p class="mt-1 text-sm text-muted">
          {[t('timeline.count.actions', { count: news.actions }), t('timeline.count.pages', { count: news.pages }), t('timeline.count.problems', { count: news.problems })].join(' · ')}
        </p>
        {#if news.waiting > 0}<p class="mt-1 text-sm font-semibold text-accent">{t('timeline.count.waiting', { count: news.waiting })}</p>{/if}
      {/if}
    </section>

    <div class="mt-4 flex flex-wrap items-end gap-2">
      <label class="min-w-0 flex-1 text-sm font-semibold">
        {t('timeline.filter')}
        <select class="{field} mt-1" bind:value={filter}>
          {#each ['all', 'agent', 'people', 'problems'] as const as f (f)}<option value={f}>{t(`timeline.filter.${f}`)}</option>{/each}
        </select>
      </label>
      <Button size="sm" icon={ICONS.download} onclick={() => save(`${exported()}.md`, 'text/markdown', markdown(label || t('timeline.title'), said, time))}>{t('timeline.export.markdown')}</Button>
      <Button size="sm" variant="ghost" icon={ICONS.download} onclick={() => save(`${exported()}.json`, 'application/json', JSON.stringify({ title: label || t('timeline.title'), events }, null, 2))}>{t('timeline.export.json')}</Button>
    </div>

    {#if first > 1}<p class="mt-4 text-[13px] text-muted">{t('timeline.dropped')}</p>{/if}
    {#if said.length === 0}
      <p class="mt-4 text-sm text-muted">{t('timeline.empty')}</p>
    {:else}
      <ol class="mt-4 space-y-1">
        {#each said.filter((l) => shown(l, filter)) as l (l.event.id)}
          <li class="flex gap-3 rounded-xl px-2 py-2 {l.event.seq > seen ? 'bg-accent/5' : ''}">
            <Icon d={ACTOR_ICONS[l.actor]} class="mt-0.5 size-4 shrink-0 text-muted" />
            <div class="min-w-0 flex-1">
              <p class="text-sm {TONES[l.tone]}">
                {l.text}{#if l.times > 1}<span class="ml-1 text-muted">{t('timeline.times', { count: l.times })}</span>{/if}
              </p>
              {#if l.outcome}<p class="text-[13px] text-bad break-words">{l.outcome}</p>{/if}
              {#if l.steps && l.steps.length > 0}
                <details class="mt-1 text-[13px] text-muted">
                  <summary class="cursor-pointer">{t('timeline.steps.show')}</summary>
                  <ol class="mt-1 list-decimal pl-5">{#each l.steps as step, i (i)}<li>{step}</li>{/each}</ol>
                </details>
              {/if}
            </div>
            <span class="shrink-0 text-right text-[12px] tabular-nums text-muted">
              {time(l.event.at)}{#if l.seconds !== undefined}<br />{t('timeline.took', { seconds: l.seconds })}{/if}
            </span>
          </li>
        {/each}
      </ol>
    {/if}
  {/if}
</div>
