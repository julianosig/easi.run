<script lang="ts">
  // The administrator's sessions: the running ones (an agent token's too), to watch or close, and the saved ones
  // (persistent sessions not running now), to resume. A new session starts in a dialog that stays out of the way until
  // asked for; a persistent one is a saved profile underneath (as the API calls it), made first and started from.
  import { followLists } from './updates.ts';
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
  import ProxyCheck from './ProxyCheck.svelte';
  import { BUSY, type Check, type CheckState, checkKey, isCheckCode, type Proxy, type ProxyList } from './proxies.ts';
  import { LOCAL, type ProfileInfo, type ProfileList, toggled } from './profiles.ts';
  import Runs from './Runs.svelte';
  import Saved from './Saved.svelte';
  import Switch from './Switch.svelte';
  import { field, focusRing, textLink } from './ui.ts';
  import { helpReason } from './timeline.ts';
  import { tick } from 'svelte';
  import { SCREENS, screenFor } from './screens.ts';
  import { ALL_FEATURES, type Features } from '../features.ts';
  import Skeleton from './Skeleton.svelte';

  let {
    onWatch,
    onLogin,
    onAgents,
    onProxies,
  }: { onWatch: (id: string) => void; onLogin: () => void; onAgents: () => void; onProxies: () => void } = $props();

  /** Where a new session opens: they (or the agent) go on from there. A saved session reopens the pages it had open. */
  const START_PAGE = 'https://www.google.com/';
  let devtools = $state(false);
  let forAgent = $state(false);
  /** `WIDTHxHEIGHT` of SCREENS, or '' for the one nearest this device's screen and window. */
  let screenSize = $state('');
  let features: Features = $state({ ...ALL_FEATURES });
  /** What a person at the session may do, each with its icon, in the order the dialog lists them; the replay is the agent's. */
  const FEATURES: [keyof Features, string][] = [
    ['keyboard_pointer', ICONS.keyboard],
    ['clipboard', ICONS.clipboard],
    ['uploads', ICONS.upload],
    ['downloads', ICONS.download],
    ['sound', ICONS.sound],
    ['recording', ICONS.record],
  ];
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
  /** Why it did not start, brought into view: the dialog may be scrolled far from it. */
  let startFailure: HTMLElement | undefined = $state();
  $effect(() => {
    if (startError) startFailure?.closest('dialog')?.scrollTo({ top: 0 });
  });
  let egresses: Proxy[] = $state([]);
  /** The check of the proxy picked to browse from (by id), run as it is picked: a session starts only through one
   * that passes. `busy`: the server runs other checks now, and Retry asks again. */
  let proxyCheck: { id: string; result: CheckState } | null = $state(null);
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

  /** Where this browser remembers the proxy a new session last started through (a saved one's is the server's). */
  const LAST_PROXY = 'easi-run:proxy:new';

  function open(resume: ProfileInfo | null) {
    startError = null;
    persistent = false;
    unlockPassword = '';
    name = '';
    screenSize = '';
    features = { ...ALL_FEATURES };
    resumeName = resume?.name ?? '';
    allowedDraft = [...(resume?.allowed ?? [])];
    opening = { resume };
    // The proxy it last started through (a saved session's, as the server remembers it, whoever started it), while there
    // is one by that id; checked again, as it is now.
    const last = resume ? resume.egress : remembered(LAST_PROXY);
    egress = last !== null && egresses.some((p) => p.id === last) ? last : '';
    forgotten = resume !== null && last !== null && egress === '';
    useProxy = egress !== '';
    if (egress !== '') void checkProxy(egress);
  }

  /** The saved session's name and agents as its dialog has them: each change is saved as it is made, started or not. */
  let resumeName = $state('');
  let allowedDraft: string[] = $state([]);
  let saving: Promise<boolean> = Promise.resolve(true);
  function saveResume(): Promise<boolean> {
    // As they are now: the dialog may be closed, or open on another one, by the time an earlier save has ended.
    const target = opening?.resume;
    const known = tokens;
    if (!target || known === null) return saving;
    const newName = resumeName.trim() || target.name;
    // Only who may still be allowed: a token revoked meanwhile is no longer one.
    const allowed = allowedDraft.filter((id) => id === LOCAL || known.some((token) => token.id === id && !token.revoked));
    saving = saving.then(async () => {
      if (newName === target.name && allowed.length === target.allowed.length && allowed.every((id) => target.allowed.includes(id))) return true;
      try {
        await api('PUT', `/api/profiles/${target.id}`, { name: newName, allowed });
        target.name = newName;
        target.allowed = allowed;
        startError = null;
        load();
        return true;
      } catch (e) {
        if (statusOf(e) === 401) failed(e);
        else startError = errorOf(e);
        return false;
      }
    });
    return saving;
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
    if (resume) {
      // Its dialog stays as it is until this ends: `starting` holds it open.
      starting = true;
      const saved = await saveResume();
      starting = false;
      if (!saved) return;
    }
    // As asked now: the dialog stays open, and these unchanged, until the start is answered.
    const [width, height] = screenSize.split('x').map(Number);
    const size = screenSize ? { width, height } : screenFor(screen.width, screen.height, innerWidth);
    // A recording is a download: it is kept only with downloads.
    const allowed = { ...features, recording: features.recording && features.downloads };
    const body: Record<string, unknown> = { url: START_PAGE, devtools, agent: forAgent, ...size, features: allowed };
    if (useProxy && egress !== '') body.egress = egress;
    // The pick and its check as sent: a refusal for them shows there only while they are still the ones shown.
    const sent = checkTurn;
    // The name as submitted: the field stays open while the saved sessions unlock.
    const profileName = name;
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
        made = await api<ProfileInfo>('POST', '/api/profiles', { name: profileName });
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
    <Saved list={profiles} onChange={load} onResume={(p) => open(p)} {onLogin} listed={shown !== 'running'} hint={shown === 'saved'} />
  {/if}
  <Runs {onLogin} />
  </div>
{/if}

<Dialog
  title={opening?.resume ? t('sessions.resume.title', { name: opening.resume.name }) : t('sessions.start.title')}
  icon={opening?.resume ? ICONS.persistent : ICONS.sessions}
  subtitle={opening?.resume ? t('sessions.resume.subtitle') : t('sessions.start.subtitle')}
  wide
  bind:open={() => opening !== null, (on) => !on && !starting && ((opening = null), checkTurn++)}
>
  {#snippet action()}
    <Button
      variant="primary"
      size="xl"
      icon={ICONS.start}
      label={opening?.resume ? t('sessions.resume') : t('sessions.start.submit')}
      type="submit"
      form="start-session"
      class="shrink-0"
      disabled={starting || !proxyReady}
    />
  {/snippet}
  <form id="start-session" onsubmit={start}>
    <p bind:this={startFailure} class="mt-3 text-sm font-semibold text-bad empty:hidden" role="alert">{startError?.()}</p>
    <!-- It opens on Google; with a proxy set up, the person picks where websites see it from, checked as picked. -->
    {#if forgotten && egress === ''}
      <p class="mb-3 text-sm font-semibold text-bad" role="status">
        {t('sessions.start.forgotten')}
      </p>
    {/if}
    <!-- Three parts, each in two even columns: the browser; what a person at it may do; what concerns the agents. -->
    <div class="mt-5 -mx-3 grid items-start gap-x-2 md:grid-cols-2 [&>*]:min-w-0">
      {#if opening?.resume}
        <div class="flex items-center gap-3 rounded-xl px-3 py-1.5">
          <span class="grid size-8 shrink-0 place-items-center rounded-xl bg-accent-soft text-accent"><Icon d={ICONS.name} class="size-[18px]" /></span>
          <span class="min-w-0 flex-1">
            <label class="block text-sm font-semibold" for="resume-name">{t('sessions.name')}</label>
            <span class="block text-[13px] leading-snug text-muted">{t('sessions.resume.name.help')}</span>
          </span>
          <input id="resume-name" data-optional class="w-36 shrink-0 rounded-lg bg-wash px-3 py-2 text-sm font-semibold text-ink {focusRing}" type="text" required bind:value={resumeName} onchange={saveResume} />
        </div>
      {/if}
      {#if !opening?.resume}
        <!-- On, the switch and what it needs share one card, the fields set in under its words: they belong to it. -->
        <div class={persistent ? 'rounded-xl bg-wash pb-4' : undefined}>
        <Switch dense icon={ICONS.persistent} title={t('sessions.start.persistent')} help={t('sessions.start.persistent.help')} bind:checked={persistent} disabled={!canPersist}>
          {#if profiles && profiles.profiles.length >= profiles.max}<span class="mt-1 block text-[13px] font-semibold text-ink">{t('sessions.start.persistent.full', { count: profiles.max })}</span>{/if}
        </Switch>
        {#if persistent}
          <div class="pl-14 pr-3 [&>*:first-child]:mt-1">
            {#if profiles && !profiles.unlocked}
              <!-- Locked since the server started: the password opens them, here, as the session starts. -->
              <Field icon={ICONS.password} label={t('sessions.unlock.password')} for="unlock-to-start" help={t('sessions.start.persistent.unlock')}>
                <input id="unlock-to-start" class={field} type="password" autocomplete="current-password" placeholder={t('sessions.unlock.password')} required aria-describedby="unlock-to-start-help" bind:value={unlockPassword} />
              </Field>
            {/if}
            <Field icon={ICONS.name} label={t('sessions.name')} for="session-name" help={t('sessions.name.help')}>
              <input id="session-name" class={field} type="text" placeholder={t('sessions.name.placeholder')} required aria-describedby="session-name-help" bind:value={name} />
            </Field>
          </div>
        {/if}
        </div>
      {/if}
        <div class="flex items-center gap-3 rounded-xl px-3 py-1.5">
          <span class="grid size-8 shrink-0 place-items-center rounded-xl bg-accent-soft text-accent"><Icon d={ICONS.screen} class="size-[18px]" /></span>
          <span class="min-w-0 flex-1">
            <label class="block text-sm font-semibold" for="start-screen">{t('sessions.start.screen')}</label>
            <span class="block text-[13px] leading-snug text-muted">{t('sessions.start.screen.help')}</span>
          </span>
          <span class="relative shrink-0">
            <select id="start-screen" data-optional class="choice cursor-pointer rounded-lg bg-wash py-2 pl-3 pr-8 text-sm font-semibold text-ink transition hover:bg-accent-soft {focusRing}" bind:value={screenSize}>
              <option value="">{t('sessions.start.screen.fit')}</option>
              {#each SCREENS as [w, h] (`${w}x${h}`)}<option value="{w}x{h}">{t('sessions.start.screen.size', { width: String(w), height: String(h) })}</option>{/each}
            </select>
            <Icon d={ICONS.collapse} class="pointer-events-none absolute right-2.5 top-1/2 size-4 -translate-y-1/2 text-muted" />
          </span>
        </div>
      {#if egresses.length > 0}
        <!-- On, the proxy to browse through is searched for and picked here, and checked as picked. -->
        <div class={useProxy ? 'rounded-xl bg-wash pb-4' : undefined}>
          <Switch dense icon={ICONS.proxies} title={t('sessions.start.proxy')} help={t('sessions.start.proxy.help')} checked={useProxy} onchange={proxyOn} />
          {#if useProxy}
            <div class="mt-1 pl-14 pr-3">
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
                <ProxyCheck state={result} checking={t('sessions.check.checking')} passed={(latency, exit) => (exit ? t('sessions.check.connected.exit', { exit, latency }) : t('sessions.check.connected', { latency }))} />
                {#if result === 'busy' || (result !== 'checking' && !result.ok)}
                  <button class={textLink} type="button" onclick={() => checkProxy(egress)}>{t('sessions.check.retry')}</button>
                  {#if result !== 'busy'}<button class={textLink} type="button" onclick={onProxies}>{t('sessions.check.proxies')}</button>{/if}
                {/if}
              </p>
            {/if}
            </div>
          {/if}
        </div>
      {/if}
    </div>
    <hr class="mt-5 -mx-6 border-line sm:-mx-8" />
    <section class="mt-5 -mx-3 grid gap-x-2 md:grid-cols-2 [&>*]:min-w-0" aria-label={t('sessions.start.features')}>
      {#each FEATURES as [key, icon] (key)}
        <Switch
          {icon}
          dense
          title={t(`sessions.start.feature.${key}`)}
          help={key === 'recording' && !features.downloads ? t('sessions.start.feature.recording.needs') : t(`sessions.start.feature.${key}.help`)}
          checked={features[key] && (key !== 'recording' || features.downloads)}
          disabled={key === 'recording' && !features.downloads}
          onchange={(on) => (features[key] = on)}
        />
      {/each}
    </section>
    <hr class="mt-5 -mx-6 border-line sm:-mx-8" />
    <section class="mt-5" aria-label={t('sessions.agents')}>
      <div class="-mx-3 grid gap-x-2 md:grid-cols-2 [&>*]:min-w-0">
        <Switch dense icon={ICONS.devtools} title={t('sessions.start.devtools')} help={t('sessions.start.devtools.help')} bind:checked={devtools} />
        <Switch dense icon={ICONS.watch} title={t('sessions.start.forAgent')} help={t('sessions.start.forAgent.help')} bind:checked={forAgent} />
        <Switch dense icon={ICONS.replay} title={t('sessions.start.feature.replay')} help={t('sessions.start.feature.replay.help')} checked={features.replay} onchange={(on) => (features.replay = on)} />
        {#if opening?.resume}
          <Switch
            dense
            icon={ICONS.terminal}
            title={t('sessions.agents.local')}
            help={t('sessions.agents.local.help')}
            checked={allowedDraft.includes(LOCAL)}
            onchange={(on) => ((allowedDraft = toggled(allowedDraft, LOCAL, on)), void saveResume())}
          />
          {#each (tokens ?? []).filter((token) => !token.revoked) as token (token.id)}
            <Switch dense icon={ICONS.token} title={token.name} help={t('sessions.agents.token.help')} checked={allowedDraft.includes(token.id)} onchange={(on) => ((allowedDraft = toggled(allowedDraft, token.id, on)), void saveResume())} />
          {/each}
        {/if}
      </div>
      {#if opening?.resume && tokens !== null && tokens.length === 0}
        <p class="mt-3 text-sm text-muted">{t('sessions.agents.none.before')}<button class={textLink} type="button" onclick={onAgents}>{t('sessions.agents.none.link')}</button>{t('sessions.agents.none.after')}</p>
      {/if}
    </section>
  </form>
</Dialog>
