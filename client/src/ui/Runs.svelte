<script lang="ts">
  // The sessions that stopped lately and whose timelines the admin keeps (`GET /api/runs`): each opens in its timeline
  // and replay, as a running session's does on its page, and can be forgotten now.
  import Button from './Button.svelte';
  import Icon from './Icon.svelte';
  import Replay from './Replay.svelte';
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { api, type Run, statusOf } from './api.ts';
  import { errorOf } from './errors.ts';

  let { onLogin }: { onLogin: () => void } = $props();

  let runs: Run[] = $state([]);
  /** The run whose timeline and replay are open over the page. */
  let open: Run | null = $state(null);
  let failure: Said | null = $state(null);

  // The latest load's answer only: one asked before a forget (or another load) changes nothing once it comes.
  let turn = 0;
  async function load() {
    const mine = ++turn;
    try {
      const listed = (await api<{ runs: Run[] }>('GET', '/api/runs')).runs;
      if (mine !== turn) return;
      runs = listed;
      failure = null;
    } catch (e) {
      if (mine !== turn) return;
      if (statusOf(e) === 401) return onLogin();
      failure = errorOf(e);
    }
  }
  // Again every few seconds: a session stopped from this page (or by its agent) joins the list.
  $effect(() => {
    void load();
    const timer = setInterval(load, 10_000);
    return () => clearInterval(timer);
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
  const time = (ms: number) => new Date(ms).toLocaleString(i18n.locale, { dateStyle: 'short', timeStyle: 'short' });
</script>

{#if runs.length > 0 || failure}
  <section class="mt-10" aria-labelledby="runs-title">
    <h2 id="runs-title" class="text-[17px] font-semibold tracking-tight">{t('timeline.runs.title')}</h2>
    <p class="mt-1 text-sm text-muted">{t('timeline.runs.subtitle')}</p>
    <p class="mt-2 text-sm font-semibold text-bad empty:hidden" role="alert">{failure?.()}</p>
    <ul class="mt-3 divide-y divide-line">
      {#each runs as run (run.id)}
        <li class="flex items-center gap-4 py-3">
          <Icon d={ICONS.timeline} class="size-5 shrink-0 text-muted" />
          <span class="min-w-0 flex-1">
            <span class="block truncate font-semibold">{run.label}</span>
            <span class="block text-sm text-muted">{t('timeline.run.stopped', { time: time(run.stopped) })}</span>
          </span>
          <Button size="sm" icon={ICONS.timeline} onclick={() => (open = run)}>{t('timeline.runs.open')}</Button>
          <Button variant="ghost" tone="danger" size="sm" icon={ICONS.delete} label={t('timeline.runs.forget')} onclick={() => forget(run)} />
        </li>
      {/each}
    </ul>
  </section>
{/if}

{#if open}
  {#key open.id}<Replay id={open.id} label={open.label} live={false} at={null} onClose={() => (open = null)} {onLogin} />{/key}
{/if}
