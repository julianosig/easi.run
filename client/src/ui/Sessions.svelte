<script lang="ts">
  // The administrator's sessions: the running ones (an agent token's too), to watch or close, and the saved ones
  // (persistent sessions not running now), to resume. A new session, or a saved one resumed, starts in StartSession's
  // dialog, which stays out of the way until asked for.
  import { followLists } from './updates.ts';
  import Button from './Button.svelte';
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { CHIP_TONES, chip, row, rowBadge, rows } from './admin.ts';
  import { api, type SessionInfo, type TokenInfo } from './api.ts';
  import { saidOf } from './errors.ts';
  import { requests } from './requests.svelte.ts';
  import EmptyState from './EmptyState.svelte';
  import Icon from './Icon.svelte';
  import PageHeader from './PageHeader.svelte';
  import type { Proxy, ProxyList } from './proxies.ts';
  import type { ProfileList } from './profiles.ts';
  import Runs from './Runs.svelte';
  import Saved from './Saved.svelte';
  import StartSession from './StartSession.svelte';
  import { focusRing } from './ui.ts';
  import { helpReason } from './timeline.ts';
  import Skeleton from './Skeleton.svelte';

  let {
    onWatch,
    onLogin,
    onAgents,
    onProxies,
  }: { onWatch: (id: string) => void; onLogin: () => void; onAgents: () => void; onProxies: () => void } = $props();

  let egresses: Proxy[] = $state([]);
  let profiles: ProfileList | null = $state(null);
  /** The agents' tokens; null until they are read. */
  let tokens: TokenInfo[] | null = $state(null);
  let sessions: SessionInfo[] | null = $state(null);
  let error: Said | null = $state(null);
  let starter: StartSession | undefined = $state();
  // A response that arrives after this view is gone must not navigate.
  const page = requests(() => onLogin());
  const alive = page.alive;
  /** Answers a failed request on the page. */
  const failed = (e: unknown) => page.fail(e, (said) => (error = said));

  /** The sessions, the saved ones (with the tokens that name their agents), and the proxies to browse from: asked at
   * once, as each answer costs a round trip, and shown together (a saved session resumed before the proxies came would
   * find its proxy gone). */
  async function load() {
    try {
      const answers = await page.latest(() =>
        Promise.all([
          api<{ sessions: SessionInfo[] }>('GET', '/api/sessions'),
          api<ProfileList>('GET', '/api/profiles'),
          api<{ tokens: TokenInfo[] }>('GET', '/api/tokens'),
          api<ProxyList>('GET', '/api/egresses'),
        ]),
      );
      if (!answers) return;
      const [listed, saved, agents, proxies] = answers;
      [sessions, profiles, tokens, egresses] = [listed.sessions, saved, agents.tokens, proxies.egresses];
    } catch (e) {
      failed(e);
    }
  }
  // Everything again when it changes: an agent may ask for you, or stop a saved session, meanwhile.
  $effect(() => followLists(() => void load()));
  $effect(() => {
    load();
  });


  async function close(id: string) {
    error = null;
    await api('DELETE', `/api/sessions/${id}`).catch(failed);
    load();
  }

  /** A running session's saved entry, when it is persistent. */
  const savedOf = (s: SessionInfo) => (s.profile ? (profiles?.profiles.find((p) => p.id === s.profile?.id) ?? null) : null);
  /** The proxy a session browses from, by its name now (or the one it had when the session started). */
  const proxyName = (s: SessionInfo) => (s.egress_id ? (egresses.find((e) => e.id === s.egress_id)?.name ?? s.egress ?? null) : null);
  /** What the list shows: everything (running first), what runs, or the saved sessions (running or not). */
  type View = 'all' | 'running' | 'saved';
  let view = $state<View>('all');
  const waiting = $derived.by(() => profiles?.profiles.filter((p) => p.session === null).length ?? 0);
  const views = $derived.by((): [View, number][] => [
    ['all', (sessions?.length ?? 0) + waiting],
    ['running', sessions?.length ?? 0],
    ['saved', profiles?.profiles.length ?? 0],
  ]);
  /** The filter shows only when one of its choices narrows the list (has some of it, not all), and then only the choices
   * that have something; otherwise everything is listed. */
  const choices = $derived.by(() => views.filter(([key, count]) => key === 'all' || count > 0));
  const filtering = $derived.by(() => views.some(([key, count]) => key !== 'all' && count > 0 && count < views.find(([k]) => k === 'all')![1]));
  const shown = $derived.by((): View => (filtering && choices.some(([key]) => key === view) ? view : 'all'));
  const running = $derived.by(() => (sessions === null ? [] : shown === 'saved' ? sessions.filter((s) => s.profile) : sessions));
  // Nothing at all to list; locked saved sessions still show, for their Unlock.
  const empty = $derived.by(() => sessions !== null && sessions.length === 0 && profiles !== null && profiles.unlocked && profiles.profiles.length === 0);
</script>

<PageHeader icon={ICONS.sessions} title={t('sessions.title')} subtitle={t('sessions.subtitle')} add={sessions ? { label: t('sessions.new'), onclick: () => starter?.open(null) } : undefined} />

<p class="mt-6 font-semibold text-bad empty:hidden" role="alert">{error?.()}</p>
{#if profiles?.leftovers}<p class="mt-4 text-sm text-bad" role="status">{t('sessions.leftovers', { count: profiles.leftovers })}</p>{/if}

{#if sessions === null}
  {#if !error}<Skeleton said={t('sessions.loading')} />{/if}
{:else if empty}
  <EmptyState points={[t('sessions.empty.persistent'), t('sessions.empty.agents')]} />
{:else}
  <!-- With no filter above them, the lists start where it would have. -->
  <div class={filtering ? undefined : 'mt-6'}>
  {#if filtering}
  <div class="mt-10 flex flex-wrap gap-2" role="group" aria-label={t('sessions.filter')}>
    {#each choices as [key, count] (key)}
      <button
        class="rounded-full px-3.5 py-1.5 text-sm font-semibold transition {focusRing} {shown === key ? 'bg-accent-soft text-accent' : 'text-muted hover:bg-wash hover:text-ink'}"
        type="button"
        aria-pressed={shown === key}
        onclick={() => (view = key)}
      >{t(`sessions.filter.${key}`)} <span class="ml-0.5 tabular-nums opacity-70">{count}</span></button>
    {/each}
  </div>
  {/if}
  {#if running.length > 0}
    <!-- What runs comes first, each with its status. -->
    <ul class="mt-4 {rows}" aria-label={t('sessions.running')}>
        {#each running as s (s.id)}
          {@const saved = savedOf(s)}
          {@const via = proxyName(s)}
          <li class="flex items-center gap-4 py-3 {row}">
            <span class={rowBadge}>
              <Icon d={s.profile ? ICONS.persistent : ICONS.website} class="size-5" />
              <span class="absolute -right-0.5 -top-0.5 size-3 rounded-full ring-2 ring-paper {s.failed ? 'bg-bad' : 'bg-good'}" aria-hidden="true"></span>
            </span>
            <span class="min-w-0 flex-1">
              <span class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                <span class="truncate font-semibold">{s.profile ? s.profile.name : s.url}</span>
                {#if s.failed}<span class="{chip} {CHIP_TONES.bad}">{t('sessions.status.failed')}</span>{/if}
                {#if s.help}<span class="{chip} {CHIP_TONES.accent}">{t('sessions.status.needsYou')}</span>{/if}
              </span>
              <!-- Its line, as a saved session's: where it is (a saved one's page), where it browses from, its tools. -->
              <span class="mt-1 block truncate text-sm text-muted"
                >{[via ? t('sessions.summary.via', { proxy: via }) : t('sessions.summary.here'), s.devtools ? t('sessions.chip.devtools') : null]
                  .filter(Boolean)
                  .join(' · ')}</span
              >
              {#if s.failed}<span class="mt-1 block text-sm font-semibold text-bad">{saidOf(s.failed)()}</span>{/if}
              {#if s.help}<span class="mt-1 block text-sm font-semibold text-accent"
                  >{t(`session.help.${helpReason(s.help.reason)}`)} · {t('session.help.since', { time: new Date(s.help.since).toLocaleTimeString(i18n.locale, { hour: '2-digit', minute: '2-digit' }) })}</span
                >{/if}
              {#if saved?.unsaved}<span class="mt-1 block text-sm font-semibold text-bad">{saidOf(saved.unsaved)()}</span>{/if}
            </span>
            <Button size="sm" icon={ICONS.watch} onclick={() => onWatch(s.id)}>{t('sessions.watch')}</Button>
            <!-- As wide as a saved row's two, so the lists' actions line up. -->
            <span class="flex w-[4.75rem] shrink-0 justify-end">
              {#if s.profile}
                <Button variant="ghost" tone="danger" size="sm" icon={ICONS.close} label={t('sessions.close', { name: s.profile.name })} onclick={() => close(s.id)} />
              {:else}
                <!-- Nothing of it is kept: ending it deletes it, asked first. -->
                <Button variant="ghost" tone="danger" size="sm" icon={ICONS.delete} label={t('sessions.discard', { name: s.url })} onclick={() => confirm(t('sessions.discard.confirm', { name: s.url })) && close(s.id)} />
              {/if}
            </span>
          </li>
        {/each}
    </ul>
  {:else if shown === 'running'}
    <p class="mt-4 text-sm text-muted">{t('sessions.running.none')}</p>
  {/if}

  {#if profiles}
    <Saved list={profiles} onChange={load} onResume={(p) => starter?.open(p)} {onLogin} listed={shown !== 'running'} hint={shown === 'saved'} />
  {/if}
  <Runs {onLogin} />
  </div>
{/if}

<StartSession bind:this={starter} bind:profiles {egresses} {tokens} {alive} {failed} onStarted={onWatch} onUnstarted={(said) => (error = said)} onChange={load} {onAgents} {onProxies} />
