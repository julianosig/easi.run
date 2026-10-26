<script lang="ts">
  // The administrator's sessions: the running ones (an agent token's too), to watch or close, and the saved ones
  // (persistent sessions not running now), to resume. A new session starts in a dialog that stays out of the way until
  // asked for; a persistent one is a saved profile underneath (as the API calls it), made first and started from.
  import { CHIP_TONES, chip, FORM_ICONS, groupLabel, menuItem, PAGE_ICONS, PLUS_ICON, row, rowAction, rowBadge, rows } from './admin.ts';
  import { api, bodyOf, messageOf, type SessionInfo, statusOf, type TokenInfo } from './api.ts';
  import Dialog from './Dialog.svelte';
  import Field from './Field.svelte';
  import Icon from './Icon.svelte';
  import Menu from './Menu.svelte';
  import PageHeader from './PageHeader.svelte';
  import { BUSY, type Check, isCheckReason, type Proxy, type ProxyList, reasonText } from './proxies.ts';
  import type { ProfileInfo, ProfileList } from './profiles.ts';
  import Saved from './Saved.svelte';
  import Switch from './Switch.svelte';
  import { button, field, primary } from './ui.ts';
  import Skeleton from './Skeleton.svelte';

  let {
    onWatch,
    onLogin,
    onAgents,
    onProxies,
  }: { onWatch: (id: string) => void; onLogin: () => void; onAgents: () => void; onProxies: () => void } = $props();

  /** Where a session the person starts opens: they (or the agent) go on from there. */
  const START_PAGE = 'https://www.google.com/';
  const WATCH_ICON = 'M2.5 12s3.5-6.5 9.5-6.5 9.5 6.5 9.5 6.5-3.5 6.5-9.5 6.5S2.5 12 2.5 12zM12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z';
  let devtools = $state(false);
  let egress = $state('');
  let persistent = $state(false);
  let name = $state('');
  let egresses: Proxy[] = $state([]);
  /** The check of the proxy picked to browse from (by id), run as it is picked: a session starts only through one
   * that passes. `busy`: the server runs other checks now, and Retry asks again. */
  let proxyCheck: { id: string; result: Check | 'checking' | 'busy' } | null = $state(null);
  let profiles: ProfileList | null = $state(null);
  let tokens: TokenInfo[] = $state([]);
  let sessions: SessionInfo[] | null = $state(null);
  let error = $state('');
  /** What went wrong starting one, in its dialog. */
  let startError = $state('');
  let starting = $state(false);
  /** The start dialog: a new session, or a saved one resumed. */
  let opening: { resume: ProfileInfo | null } | null = $state(null);
  /** The row whose menu is open. */
  let menu: string | null = $state(null);
  // A response that arrives after this view is gone must not navigate.
  let alive = true;
  $effect(() => () => (alive = false));

  /** Answers a failed request: a lapsed login goes to the login, anything else is said on the page. */
  function failed(e: unknown): void {
    if (!alive) return;
    if (statusOf(e) === 401) return onLogin();
    error = messageOf(e);
  }

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

  function open(resume: ProfileInfo | null) {
    startError = '';
    persistent = false;
    name = '';
    opening = { resume };
    // A proxy picked before is checked again, as it is now.
    if (egress !== '') void checkProxy(egress);
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
      if (alive && mine === checkTurn) proxyCheck = { id, result };
    } catch (e) {
      if (!alive || mine !== checkTurn) return;
      if (statusOf(e) === 401) return failed(e);
      if (statusOf(e) === 503) return void (proxyCheck = { id, result: 'busy' });
      // Not checked: it cannot be used yet, and Retry asks again.
      proxyCheck = { id, result: { ok: false, reason: 'unreachable' } };
      startError = messageOf(e);
    }
  }
  /** Whether the session may start: directly, or through a proxy that just passed its check. */
  const proxyReady = $derived.by(() => {
    if (egress === '') return true;
    const c = proxyCheck;
    return c !== null && c.id === egress && typeof c.result !== 'string' && c.result.ok;
  });

  /** Why a session could not start, as the person reads it. */
  const why = (e: unknown) =>
    statusOf(e) === 423
      ? 'Saved sessions are locked: unlock them under Saved with your password.'
      : statusOf(e) === 503
        ? `${BUSY}.`
        : statusOf(e) === 424
          ? `The proxy failed its check: ${reasonText(bodyOf(e).reason)}.`
          : messageOf(e);

  async function start(event: SubmitEvent) {
    event.preventDefault();
    const resume = opening?.resume ?? null;
    // As asked now: the dialog stays open, and these unchanged, until the start is answered.
    const body: Record<string, unknown> = { url: START_PAGE, devtools };
    if (egress !== '') body.egress = egress;
    // The pick and its check as sent: a refusal for them shows there only while they are still the ones shown.
    const sent = checkTurn;
    starting = true;
    startError = '';
    // A persistent session is made first (as a saved profile), then started from.
    let made: ProfileInfo | null = null;
    if (!resume && persistent) {
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
    if (!alive) return;
    try {
      const from = resume ?? made;
      if (from) body.profile = from.id;
      const created = await api<SessionInfo>('POST', '/api/sessions', body);
      if (alive) onWatch(created.id);
    } catch (e) {
      starting = false;
      if (!alive) return;
      if (statusOf(e) === 401) return failed(e);
      if (made) {
        // Saved, but not started: it stays under Saved, to resume or delete.
        opening = null;
        error = `“${made.name}” is saved, but its browser did not start: ${why(e)} Resume it under Saved, or delete it.`;
        load();
        return;
      }
      // The proxy failed the check just before the start: its reason shows where its check does, with Retry, unless
      // another proxy was picked, or checked again, meanwhile (then it is said below, and what is shown stays).
      const reason = bodyOf(e).reason;
      const via = body.egress;
      if (statusOf(e) === 424 && typeof via === 'string' && isCheckReason(reason) && via === egress && sent === checkTurn) {
        ++checkTurn;
        const detail = bodyOf(e).detail;
        proxyCheck = { id: via, result: typeof detail === 'string' ? { ok: false, reason, detail } : { ok: false, reason } };
        return;
      }
      startError = why(e);
    }
  }

  async function close(id: string) {
    menu = null;
    error = '';
    await api('DELETE', `/api/sessions/${id}`).catch(failed);
    load();
  }

  /** A running session's saved entry, when it is persistent. */
  const savedOf = (s: SessionInfo) => (s.profile ? (profiles?.profiles.find((p) => p.id === s.profile?.id) ?? null) : null);
  /** The proxy a session browses from, by its name now (or the one it had when the session started). */
  const proxyName = (s: SessionInfo) => (s.egress_id ? (egresses.find((e) => e.id === s.egress_id)?.name ?? s.egress ?? null) : null);
  const canPersist = $derived.by(() => profiles !== null && profiles.unlocked && profiles.profiles.length < profiles.max);
  // Nothing at all to list; locked saved sessions still show, for their Unlock.
  const empty = $derived.by(() => sessions !== null && sessions.length === 0 && profiles !== null && profiles.unlocked && profiles.profiles.length === 0);
</script>

{#snippet plus()}
  <Icon d={PLUS_ICON} class="size-4" />
{/snippet}

<PageHeader icon={PAGE_ICONS.sessions} title="Sessions" subtitle="Your browsers and your agents’. Watch one to see it live and take control.">
  {#snippet action()}
    {#if sessions && !empty}
      <button class="{primary} inline-flex items-center gap-2" type="button" onclick={() => open(null)}>{@render plus()}New session</button>
    {/if}
  {/snippet}
</PageHeader>

<p class="mt-6 font-semibold text-bad empty:hidden" role="alert">{error}</p>

{#if sessions === null}
  {#if !error}<Skeleton said="Loading your sessions" />{/if}
{:else if empty}
  <div class="mt-20 text-center">
    <span class="mx-auto mb-4 grid size-14 place-items-center rounded-2xl bg-accent-soft text-accent"><Icon d={FORM_ICONS.website} class="size-7" /></span>
    <p class="text-lg font-semibold">No sessions yet</p>
    <p class="mx-auto mt-1.5 max-w-sm text-sm text-muted">Start a browser here; your agents start their own with a token.</p>
    <button class="{primary} mt-6 inline-flex items-center gap-2" type="button" onclick={() => open(null)}>{@render plus()}New session</button>
  </div>
{:else}
  <section class="mt-10" aria-labelledby="running-title">
    <h2 id="running-title" class={groupLabel}>Running</h2>
    {#if sessions.length === 0}
      <p class="mt-3 text-sm text-muted">Nothing running.</p>
    {:else}
      <ul class="mt-3 {rows}">
        {#each sessions as s (s.id)}
          {@const saved = savedOf(s)}
          {@const via = proxyName(s)}
          <li class="flex items-center gap-4 py-3 {row}">
            <span class={rowBadge}>
              <Icon d={s.profile ? FORM_ICONS.persistent : FORM_ICONS.website} class="size-5" />
              <span class="absolute -right-0.5 -top-0.5 size-3 rounded-full ring-2 ring-paper {s.failed ? 'bg-bad' : 'bg-good'}" aria-hidden="true"></span>
            </span>
            <span class="min-w-0 flex-1">
              <span class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                <span class="truncate font-semibold">{s.profile ? s.profile.name : s.url}</span>
                {#if s.failed}<span class="{chip} {CHIP_TONES.bad}">Failed</span>{:else}<span class="{chip} {CHIP_TONES.good}">Running</span>{/if}
              </span>
              <span class="mt-1 flex min-w-0 flex-wrap items-center gap-1.5 text-sm text-muted">
                {#if s.profile}<span class="{chip} {CHIP_TONES.accent}">Persistent</span><span class="min-w-0 truncate">{s.url}</span>{/if}
                {#if via}<span class="{chip} {CHIP_TONES.neutral}">via {via}</span>{/if}
                {#if s.devtools}<span class="{chip} {CHIP_TONES.neutral}">Developer tools</span>{/if}
              </span>
              {#if s.failed}<span class="mt-1 block text-sm font-semibold text-bad">{s.failed}</span>{/if}
              {#if saved?.unsaved}<span class="mt-1 block text-sm font-semibold text-bad">Last save failed: {saved.unsaved}</span>{/if}
            </span>
            <button class={rowAction} type="button" onclick={() => onWatch(s.id)}
              ><Icon d={WATCH_ICON} class="size-4 text-accent" />Watch</button
            >
            <Menu label="More for {s.profile ? s.profile.name : s.url}" bind:open={() => menu === s.id, (on) => (menu = on ? s.id : null)}>
              <button class="{menuItem} text-bad" type="button" onclick={() => close(s.id)}>Close session</button>
            </Menu>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  {#if profiles}
    <Saved list={profiles} {tokens} onChange={load} onResume={(p) => open(p)} {onLogin} {onAgents} />
  {/if}
{/if}

<Dialog
  title={opening?.resume ? `Resume “${opening.resume.name}”` : 'New browser session'}
  icon={opening?.resume ? FORM_ICONS.persistent : PAGE_ICONS.sessions}
  subtitle={opening?.resume ? 'Opens it with the logins and settings it saved.' : 'A private Chrome your agents can use and you can watch.'}
  bind:open={() => opening !== null, (on) => !on && !starting && ((opening = null), checkTurn++)}
>
  <form onsubmit={start}>
    <!-- It opens on Google; with a proxy set up, the person picks where websites see it from, checked as picked. -->
    {#if egresses.length > 0}
      <Field icon={PAGE_ICONS.proxies} label="Browse from" for="egress" help="Where websites see this browser connecting from.">
        <select id="egress" class={field} aria-describedby="egress-help" value={egress} onchange={(e) => checkProxy(e.currentTarget.value)}>
          <option value="">This server</option>
          {#each egresses as e (e.id)}<option value={e.id}>{e.name}</option>{/each}
        </select>
      </Field>
      {#if egress !== '' && proxyCheck?.id === egress}
        {@const result = proxyCheck.result}
        <p class="mt-2 flex flex-wrap items-center gap-2 text-sm" role="status">
          {#if result === 'checking'}
            <span class="{chip} {CHIP_TONES.neutral}"><span class="size-2.5 animate-spin rounded-full border-2 border-muted border-t-transparent" aria-hidden="true"></span>Checking the proxy…</span>
          {:else if result === 'busy'}
            <span class="{chip} {CHIP_TONES.neutral}">{BUSY}</span>
            <button class="font-semibold text-accent hover:underline" type="button" onclick={() => checkProxy(egress)}>Retry</button>
          {:else if result.ok}
            <span class="{chip} {CHIP_TONES.good}"><span class="size-1.5 rounded-full bg-good" aria-hidden="true"></span>Connected · {result.latency_ms} ms</span>
          {:else}
            <span class="{chip} {CHIP_TONES.bad}"><span class="size-1.5 rounded-full bg-bad" aria-hidden="true"></span>{reasonText(result.reason)}</span>
            <button class="font-semibold text-accent hover:underline" type="button" onclick={() => checkProxy(egress)}>Retry</button>
            <button class="font-semibold text-accent hover:underline" type="button" onclick={onProxies}>Open Proxies</button>
          {/if}
        </p>
      {/if}
    {/if}
    <div class="mt-5 -mx-3 space-y-0.5">
      {#if !opening?.resume}
        <Switch icon={FORM_ICONS.persistent} title="Persistent" help="Remembers logins, history and settings for next time — encrypted with your password." bind:checked={persistent} disabled={!canPersist}>
          {#if profiles && !profiles.unlocked}<span class="mt-1 block text-[13px] font-semibold text-ink">Unlock your saved sessions first, under Saved.</span>
          {:else if profiles && profiles.profiles.length >= profiles.max}<span class="mt-1 block text-[13px] font-semibold text-ink">At most {profiles.max} saved sessions: delete one first.</span>{/if}
        </Switch>
        {#if persistent}
          <div class="pb-4">
            <Field icon={FORM_ICONS.name} label="Name" for="session-name">
              <input id="session-name" class={field} type="text" maxlength="64" placeholder="e.g. LinkedIn" required bind:value={name} />
            </Field>
          </div>
        {/if}
      {/if}
      <Switch icon={FORM_ICONS.devtools} title="Developer tools for the agent" help="Lets the agent switch tabs, open pages and read their contents directly. Stays inside this session." bind:checked={devtools} />
    </div>
    <p class="mt-4 text-sm font-semibold text-bad empty:hidden" role="alert">{startError}</p>
    <div class="mt-7 flex justify-end gap-3">
      <button class={button} type="button" disabled={starting} onclick={() => ((opening = null), checkTurn++)}>Cancel</button>
      <button class="{primary} inline-flex items-center gap-2" type="submit" disabled={starting || !proxyReady}
        ><Icon d={FORM_ICONS.start} class="shrink-0 size-[18px]" />{opening?.resume ? 'Resume' : 'Start session'}</button
      >
    </div>
  </form>
</Dialog>
