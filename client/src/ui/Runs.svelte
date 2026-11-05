<script lang="ts">
  // The sessions that stopped and whose archives the admin keeps (`GET /api/runs`): each opens in its timeline and replay,
  // as a running session's does on its page, and can be forgotten now. After a restart only the password opens them.
  import { followLists } from './updates.ts';
  import Button from './Button.svelte';
  import Dialog from './Dialog.svelte';
  import Field from './Field.svelte';
  import Icon from './Icon.svelte';
  import Replay from './Replay.svelte';
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { api, type Run, statusOf } from './api.ts';
  import { errorOf } from './errors.ts';
  import { field } from './ui.ts';

  let { onLogin }: { onLogin: () => void } = $props();

  let runs: Run[] = $state([]);
  /** How many archives only the password opens. */
  let locked = $state(0);
  let unlocking = $state(false);
  let password = $state('');
  let unlockError: Said | null = $state(null);
  let busy = $state(false);
  /** The run whose timeline and replay are open over the page. */
  let open: Run | null = $state(null);
  let failure: Said | null = $state(null);

  // The latest load's answer only: one asked before a forget (or another load) changes nothing once it comes.
  let turn = 0;
  async function load() {
    const mine = ++turn;
    try {
      const listed = await api<{ runs: Run[]; locked: number }>('GET', '/api/runs');
      if (mine !== turn) return;
      [runs, locked] = [listed.runs, listed.locked];
      failure = null;
    } catch (e) {
      if (mine !== turn) return;
      if (statusOf(e) === 401) return onLogin();
      failure = errorOf(e);
    }
  }
  // Again when it changes: a session stopped from this page (or by its agent) joins the list.
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
  async function unlock(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    try {
      await api('POST', '/api/profiles/unlock', { password });
      [password, unlocking, unlockError] = ['', false, null];
      await load();
    } catch (e) {
      if (statusOf(e) === 401) return onLogin();
      unlockError = errorOf(e);
    } finally {
      busy = false;
    }
  }
  const time = (ms: number) => new Date(ms).toLocaleString(i18n.locale, { dateStyle: 'short', timeStyle: 'short' });
</script>

{#if runs.length > 0 || locked > 0 || failure}
  <section class="mt-10" aria-labelledby="runs-title">
    <h2 id="runs-title" class="text-[17px] font-semibold tracking-tight">{t('timeline.runs.title')}</h2>
    <p class="mt-1 text-sm text-muted">{t('timeline.runs.subtitle')}</p>
    <p class="mt-2 text-sm font-semibold text-bad empty:hidden" role="alert">{failure?.()}</p>
    {#if locked > 0}
      <div class="mt-4 flex flex-wrap items-center justify-between gap-4 rounded-2xl bg-wash px-5 py-4">
        <p class="max-w-lg text-sm text-muted">{t('timeline.runs.locked')}</p>
        <Button variant="primary" icon={ICONS.unlock} onclick={() => ((unlockError = null), (unlocking = true))}>{t('sessions.unlock')}</Button>
      </div>
    {/if}
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

<Dialog title={t('sessions.unlock')} icon={ICONS.lock} subtitle={t('timeline.runs.locked')} bind:open={() => unlocking, (v) => (unlocking = v)}>
  <form onsubmit={unlock}>
    <Field icon={ICONS.password} label={t('sessions.unlock.password')} for="runs-unlock-password">
      <input id="runs-unlock-password" class={field} type="password" autocomplete="current-password" bind:value={password} />
    </Field>
    <p class="mt-4 text-sm font-semibold text-bad empty:hidden" role="alert">{unlockError?.()}</p>
    <div class="mt-7 flex justify-end gap-3">
      <Button disabled={busy} onclick={() => (unlocking = false)}>{t('sessions.cancel')}</Button>
      <Button variant="primary" icon={ICONS.unlock} type="submit" disabled={busy}>{t('sessions.unlock')}</Button>
    </div>
  </form>
</Dialog>

{#if open}
  {#key open.id}<Replay id={open.id} label={open.label} live={false} at={null} onClose={() => (open = null)} {onLogin} />{/key}
{/if}
