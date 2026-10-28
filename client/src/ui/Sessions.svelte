<script lang="ts">
  // The administrator's sessions: the running ones (an agent token's too), to watch or close, and the saved ones
  // (persistent sessions not running now), to resume. A new session starts in a dialog that stays out of the way until
  // asked for; a persistent one is a saved profile underneath (as the API calls it), made first and started from.
  import Button from './Button.svelte';
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { CHIP_TONES, chip, row, rowBadge, rows } from './admin.ts';
  import { api, bodyOf, codeOf, remember, remembered, type SessionInfo, statusOf, type TokenInfo } from './api.ts';
  import { errorOf, saidOf } from './errors.ts';
  import { requests } from './requests.svelte.ts';
  import Combobox from './Combobox.svelte';
  import Dialog from './Dialog.svelte';
  import Field from './Field.svelte';
  import EmptyState from './EmptyState.svelte';
  import Icon from './Icon.svelte';
  import PageHeader from './PageHeader.svelte';
  import { BUSY, type Check, checkKey, exitText, isCheckCode, type Proxy, type ProxyList } from './proxies.ts';
  import type { ProfileInfo, ProfileList } from './profiles.ts';
  import Saved from './Saved.svelte';
  import Switch from './Switch.svelte';
  import { field, focusRing, textLink } from './ui.ts';
  import { tick } from 'svelte';
  import { screenFor } from './screens.ts';
  import Skeleton from './Skeleton.svelte';

  let {
    onWatch,
    onLogin,
    onAgents,
    onProxies,
  }: { onWatch: (id: string) => void; onLogin: () => void; onAgents: () => void; onProxies: () => void } = $props();

  /** Where a session the person starts opens: they (or the agent) go on from there. */
  const START_PAGE = 'https://www.google.com/';
  let devtools = $state(false);
  let egress = $state('');
  /** Whether the session browses through a proxy (then `egress` is the one picked, '' until one is). */
  let useProxy = $state(false);
  let proxySearch: HTMLInputElement | undefined = $state();
  /** The saved session being resumed last browsed through a proxy that has since been deleted. */
  let forgotten = $state(false);
  let persistent = $state(false);
  /** The password that unlocks the saved sessions, asked here when a persistent one starts while they are locked. */
  let unlockPassword = $state('');
  // Kept no longer than it is asked for: gone once Persistent is off or the dialog closes.
  $effect(() => {
    if (!persistent || opening === null) unlockPassword = '';
  });
  let name = $state('');
  let egresses: Proxy[] = $state([]);
  /** The check of the proxy picked to browse from (by id), run as it is picked: a session starts only through one
   * that passes. `busy`: the server runs other checks now, and Retry asks again. */
  let proxyCheck: { id: string; result: Check | 'checking' | 'busy' } | null = $state(null);
  let profiles: ProfileList | null = $state(null);
  /** The agents' tokens; null until they are read. */
  let tokens: TokenInfo[] | null = $state(null);
  let sessions: SessionInfo[] | null = $state(null);
  let error: Said | null = $state(null);
  /** What went wrong starting one, in its dialog. */
  let startError: Said | null = $state(null);
  let starting = $state(false);
  /** The start dialog: a new session, or a saved one resumed. */
  let opening: { resume: ProfileInfo | null } | null = $state(null);
  // A response that arrives after this view is gone must not navigate.
  const page = requests(() => onLogin());
  const alive = page.alive;
  /** Answers a failed request on the page. */
  const failed = (e: unknown) => page.fail(e, (said) => (error = said));

  /** The sessions, the saved ones (with the tokens that name their agents), and the proxies to browse from: asked at
   * once, as each answer costs a round trip. */
  async function load() {
    try {
      await Promise.all([
        api<{ sessions: SessionInfo[] }>('GET', '/api/sessions').then((answer) => (sessions = answer.sessions)),
        api<ProfileList>('GET', '/api/profiles').then((answer) => (profiles = answer)),
        api<{ tokens: TokenInfo[] }>('GET', '/api/tokens').then((answer) => (tokens = answer.tokens)),
        api<ProxyList>('GET', '/api/egresses').then((answer) => (egresses = answer.egresses)),
      ]);
    } catch (e) {
      failed(e);
    }
  }
  $effect(() => {
    load();
  });

  /** Where this browser remembers the proxy a new session last started through (a saved one's is the server's). */
  const LAST_PROXY = 'easi-run:proxy:new';

  function open(resume: ProfileInfo | null) {
    startError = null;
    persistent = false;
    unlockPassword = '';
    name = '';
    opening = { resume };
    // The proxy it last started through (a saved session's, as the server remembers it, whoever started it), while there
    // is one by that id; checked again, as it is now.
    const last = resume ? resume.egress : remembered(LAST_PROXY);
    egress = last !== null && egresses.some((p) => p.id === last) ? last : '';
    forgotten = resume !== null && last !== null && egress === '';
    useProxy = egress !== '';
    if (egress !== '') void checkProxy(egress);
  }

  /** Use a proxy, switched: on, the only one is picked at once, or the search opens to pick one; off, none is used. */
  async function proxyOn(on: boolean) {
    useProxy = on;
    if (!on) return void checkProxy('');
    if (egresses.length === 1) return void checkProxy(egresses[0]!.id);
    await tick();
    proxySearch?.focus();
  }

  /** The latest check asked for: an earlier one's late answer (a previous pick, a closed dialog) changes nothing. */
  let checkTurn = 0;
  /** Checks the proxy picked, now. */
  async function checkProxy(id: string) {
    egress = id;
    const mine = ++checkTurn;
    if (id === '') return void (proxyCheck = null);
    proxyCheck = { id, result: 'checking' };
    try {
      const result = await api<Check>('POST', `/api/egresses/${id}/check`);
      if (alive() && mine === checkTurn) proxyCheck = { id, result };
    } catch (e) {
      if (!alive() || mine !== checkTurn) return;
      if (statusOf(e) === 401) return failed(e);
      if (statusOf(e) === 503) return void (proxyCheck = { id, result: 'busy' });
      // Not checked: it cannot be used yet, and Retry asks again.
      proxyCheck = { id, result: { ok: false, code: 'proxy-check-unreachable' } };
      startError = errorOf(e);
    }
  }
  /** Whether the session may start: directly, or through a proxy that just passed its check. */
  const proxyReady = $derived.by(() => {
    if (!useProxy) return true;
    if (egress === '') return false;
    const c = proxyCheck;
    return c !== null && c.id === egress && typeof c.result !== 'string' && c.result.ok;
  });

  /** Why a session could not start, as the person reads it. */
  const why = (e: unknown): Said =>
    statusOf(e) === 423
      ? () => t('sessions.error.locked')
      : statusOf(e) === 503
        ? () => `${t(BUSY)}.`
        : statusOf(e) === 424
          ? () => t('sessions.error.check', { reason: t(checkKey(codeOf(e))) })
          : errorOf(e);

  async function start(event: SubmitEvent) {
    event.preventDefault();
    const resume = opening?.resume ?? null;
    // As asked now: the dialog stays open, and these unchanged, until the start is answered.
    const body: Record<string, unknown> = { url: START_PAGE, devtools, ...screenFor(screen.width, screen.height, innerWidth) };
    if (useProxy && egress !== '') body.egress = egress;
    // The pick and its check as sent: a refusal for them shows there only while they are still the ones shown.
    const sent = checkTurn;
    starting = true;
    startError = null;
    // A persistent session is made first (as a saved profile), then started from; the saved sessions unlocked first
    // when they are locked.
    let made: ProfileInfo | null = null;
    if (!resume && persistent) {
      if (profiles && !profiles.unlocked) {
        try {
          await api('POST', '/api/profiles/unlock', { password: unlockPassword });
          unlockPassword = '';
          profiles.unlocked = true;
        } catch (e) {
          starting = false;
          if (statusOf(e) === 401) return failed(e);
          startError = errorOf(e);
          return;
        }
      }
      try {
        made = await api<ProfileInfo>('POST', '/api/profiles', { name });
      } catch (e) {
        starting = false;
        if (statusOf(e) === 401) return failed(e);
        startError = why(e);
        return;
      }
    }
    // Gone meanwhile: a saved entry made stays under Saved; nothing starts behind the person's back.
    if (!alive()) return;
    try {
      const from = resume ?? made;
      if (from) body.profile = from.id;
      const created = await api<SessionInfo>('POST', '/api/sessions', body);
      if (!from) remember(LAST_PROXY, (body.egress as string | undefined) ?? null);
      if (alive()) onWatch(created.id);
    } catch (e) {
      starting = false;
      if (!alive()) return;
      if (statusOf(e) === 401) return failed(e);
      if (made) {
        // Saved, but not started: it stays under Saved, to resume or delete.
        opening = null;
        const unstarted = why(e);
        error = () => t('sessions.error.unstarted', { name: made.name, why: unstarted() });
        load();
        return;
      }
      // The proxy failed the check just before the start: why shows where its check does, with Retry, unless another
      // proxy was picked, or checked again, meanwhile (then it is said below, and what is shown stays).
      const code = codeOf(e);
      const via = body.egress;
      if (statusOf(e) === 424 && typeof via === 'string' && isCheckCode(code) && via === egress && sent === checkTurn) {
        ++checkTurn;
        const detail = bodyOf(e).detail;
        proxyCheck = { id: via, result: typeof detail === 'string' ? { ok: false, code, detail } : { ok: false, code } };
        return;
      }
      startError = why(e);
    }
  }

  async function close(id: string) {
    error = null;
    await api('DELETE', `/api/sessions/${id}`).catch(failed);
    load();
  }

  /** A running session's saved entry, when it is persistent. */
  const savedOf = (s: SessionInfo) => (s.profile ? (profiles?.profiles.find((p) => p.id === s.profile?.id) ?? null) : null);
  /** The proxy a session browses from, by its name now (or the one it had when the session started). */
  const proxyName = (s: SessionInfo) => (s.egress_id ? (egresses.find((e) => e.id === s.egress_id)?.name ?? s.egress ?? null) : null);
  const canPersist = $derived.by(() => profiles !== null && profiles.profiles.length < profiles.max);
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

<PageHeader icon={ICONS.sessions} title={t('sessions.title')} subtitle={t('sessions.subtitle')} add={sessions ? { label: t('sessions.new'), onclick: () => open(null) } : undefined} />

<p class="mt-6 font-semibold text-bad empty:hidden" role="alert">{error?.()}</p>

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
              </span>
              <!-- Its line, as a saved session's: where it is (a saved one's page), where it browses from, its tools. -->
              <span class="mt-1 block truncate text-sm text-muted"
                >{[s.profile ? s.url : null, via ? t('sessions.summary.via', { proxy: via }) : t('sessions.summary.here'), s.devtools ? t('sessions.chip.devtools') : null]
                  .filter(Boolean)
                  .join(' · ')}</span
              >
              {#if s.failed}<span class="mt-1 block text-sm font-semibold text-bad">{saidOf(s.failed)()}</span>{/if}
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
    <Saved list={profiles} {tokens} onChange={load} onResume={(p) => open(p)} {onLogin} {onAgents} listed={shown !== 'running'} hint={shown === 'saved'} />
  {/if}
  </div>
{/if}

<Dialog
  title={opening?.resume ? t('sessions.resume.title', { name: opening.resume.name }) : t('sessions.start.title')}
  icon={opening?.resume ? ICONS.persistent : ICONS.sessions}
  subtitle={opening?.resume ? t('sessions.resume.subtitle') : t('sessions.start.subtitle')}
  bind:open={() => opening !== null, (on) => !on && !starting && ((opening = null), checkTurn++)}
>
  <form onsubmit={start}>
    <!-- It opens on Google; with a proxy set up, the person picks where websites see it from, checked as picked. -->
    {#if forgotten && egress === ''}
      <p class="mb-3 text-sm font-semibold text-bad" role="status">
        {t('sessions.start.forgotten')}
      </p>
    {/if}
    <div class="mt-5 -mx-3 space-y-0.5">
      {#if egresses.length > 0}
        <!-- On, the proxy to browse through is searched for and picked here, and checked as picked. -->
        <div class={useProxy ? 'rounded-xl bg-wash pb-4' : undefined}>
          <Switch icon={ICONS.proxies} title={t('sessions.start.proxy')} help={t('sessions.start.proxy.help')} checked={useProxy} onchange={proxyOn} />
          {#if useProxy}
            <div class="mt-1 pl-[3.75rem] pr-3">
              <Combobox
                id="egress"
                bind:ref={proxySearch}
                label={t('sessions.start.proxy.pick')}
                icon={ICONS.search}
                placeholder={t('sessions.start.proxy.search')}
                options={egresses.map((e) => ({ value: e.id, label: e.name }))}
                value={egress}
                empty={t('sessions.start.proxy.none')}
                onpick={checkProxy}
              />
      {#if egress !== '' && proxyCheck?.id === egress}
              {@const result = proxyCheck.result}
              <p class="mt-2 flex flex-wrap items-center gap-2 text-sm" role="status">
                {#if result === 'checking'}
                  <span class="{chip} {CHIP_TONES.neutral}"><span class="size-2.5 animate-spin rounded-full border-2 border-muted border-t-transparent" aria-hidden="true"></span>{t('sessions.check.checking')}</span>
                {:else if result === 'busy'}
                  <span class="{chip} {CHIP_TONES.neutral}">{t(BUSY)}</span>
                  <button class={textLink} type="button" onclick={() => checkProxy(egress)}>{t('sessions.check.retry')}</button>
                {:else if result.ok}
                  <span class="{chip} {CHIP_TONES.good}"><span class="size-1.5 rounded-full bg-good" aria-hidden="true"></span>{result.exit ? t('sessions.check.connected.exit', { exit: exitText(result.exit, i18n.locale), latency: result.latency_ms }) : t('sessions.check.connected', { latency: result.latency_ms })}</span>
                {:else}
                  <span class="{chip} {CHIP_TONES.bad}"><span class="size-1.5 rounded-full bg-bad" aria-hidden="true"></span>{t(checkKey(result.code))}</span>
                  <button class={textLink} type="button" onclick={() => checkProxy(egress)}>{t('sessions.check.retry')}</button>
                  <button class={textLink} type="button" onclick={onProxies}>{t('sessions.check.proxies')}</button>
                {/if}
              </p>
            {/if}
            </div>
          {/if}
        </div>
      {/if}
      {#if !opening?.resume}
        <!-- On, the switch and what it needs share one card, the fields set in under its words: they belong to it. -->
        <div class={persistent ? 'rounded-xl bg-wash pb-4' : undefined}>
        <Switch icon={ICONS.persistent} title={t('sessions.start.persistent')} help={t('sessions.start.persistent.help')} bind:checked={persistent} disabled={!canPersist}>
          {#if profiles && profiles.profiles.length >= profiles.max}<span class="mt-1 block text-[13px] font-semibold text-ink">{t('sessions.start.persistent.full', { count: profiles.max })}</span>{/if}
        </Switch>
        {#if persistent}
          <div class="pl-[3.75rem] pr-3 [&>*:first-child]:mt-1">
            {#if profiles && !profiles.unlocked}
              <!-- Locked since the server started: the password opens them, here, as the session starts. -->
              <Field icon={ICONS.password} label={t('sessions.unlock.password')} for="unlock-to-start" help={t('sessions.start.persistent.unlock')}>
                <input id="unlock-to-start" class={field} type="password" autocomplete="current-password" placeholder={t('sessions.unlock.password')} required aria-describedby="unlock-to-start-help" bind:value={unlockPassword} />
              </Field>
            {/if}
            <Field icon={ICONS.name} label={t('sessions.name')} for="session-name" help={t('sessions.name.help')}>
              <input id="session-name" class={field} type="text" maxlength="64" placeholder={t('sessions.name.placeholder')} required aria-describedby="session-name-help" bind:value={name} />
            </Field>
          </div>
        {/if}
        </div>
      {/if}
      <Switch icon={ICONS.devtools} title={t('sessions.start.devtools')} help={t('sessions.start.devtools.help')} bind:checked={devtools} />
    </div>
    <p class="mt-4 text-sm font-semibold text-bad empty:hidden" role="alert">{startError?.()}</p>
    <div class="mt-7 flex justify-end gap-3">
      <Button disabled={starting} onclick={() => ((opening = null), checkTurn++)}>{t('sessions.cancel')}</Button>
      <Button variant="primary" icon={ICONS.start} type="submit" disabled={starting || !proxyReady}>{opening?.resume ? t('sessions.resume') : t('sessions.start.submit')}</Button>
    </div>
  </form>
</Dialog>
