<script lang="ts">
  // The proxies (egresses, as the API calls them) a session may browse from: connect one (the admin checks it and saves
  // it with where its exit is, in one step); edit, test or delete one. Each is checked as the page opens, one after
  // another (the server runs only a few checks at once), so its row says whether it works now.
  import { CHIP_TONES, chip, FORM_ICONS, menuItem, PAGE_ICONS, PLUS_ICON, row, rowBadge, rows } from './admin.ts';
  import { api, bodyOf, messageOf, statusOf } from './api.ts';
  import Dialog from './Dialog.svelte';
  import Field from './Field.svelte';
  import Icon from './Icon.svelte';
  import Menu from './Menu.svelte';
  import PageHeader from './PageHeader.svelte';
  import { addressText, BUSY, type Check, exitText, flagOf, isCheckReason, languageName, NAME_PATTERN, type Proxy, type ProxyList, type ProxyOptions, reasonText, zoneName } from './proxies.ts';
  import { button, field, GATE_ICONS, primary } from './ui.ts';
  import Skeleton from './Skeleton.svelte';

  let { onLogin }: { onLogin: () => void } = $props();

  let proxies: Proxy[] | null = $state(null);
  /** Each proxy's last check here, that one is under way, or that the server was too busy to run it. */
  let checks: Record<string, Check | 'checking' | 'busy'> = $state({});
  /** The latest check asked for each proxy: an earlier one's late answer changes nothing. */
  const checkTurn: Record<string, number> = {};
  let error = $state('');
  let menu: string | null = $state(null);
  /** The dialog: a new proxy (connected and saved in one step), or one being edited. */
  let editing: { proxy: Proxy | null } | null = $state(null);
  /** Bumped as the dialog opens and closes: an answer for an earlier one changes nothing. */
  let turn = 0;
  let name = $state('');
  let address = $state('');
  /** An edited proxy's time zone and languages, picked from what the admin offers. */
  let timezone = $state('');
  let languages: string[] = $state([]);
  let options: ProxyOptions | null = $state(null);
  let saving = $state(false);
  /** Why the last connect or save failed: a check's reason in plain words, or the admin's message. */
  let dialogError = $state('');
  /** Said a moment after a proxy is connected. */
  let toast = $state('');
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  let alive = true;
  $effect(() => () => {
    alive = false;
    clearTimeout(toastTimer);
  });

  function failed(e: unknown, where: (text: string) => void = (text) => (error = text)): void {
    if (!alive) return;
    if (statusOf(e) === 401) return onLogin();
    where(messageOf(e));
  }

  async function load() {
    try {
      const list = (await api<ProxyList>('GET', '/api/egresses')).egresses;
      if (!alive) return;
      proxies = list;
      // One at a time: all at once, most would find the server busy.
      for (const p of list) {
        if (!alive) return;
        if (!(p.id in checks)) await check(p);
      }
    } catch (e) {
      failed(e);
    }
  }
  $effect(() => {
    load();
  });

  /** Checks a saved proxy now; its row shows the answer. */
  async function check(p: Proxy) {
    menu = null;
    const mine = (checkTurn[p.id] = (checkTurn[p.id] ?? 0) + 1);
    checks[p.id] = 'checking';
    try {
      const answer = await api<Check>('POST', `/api/egresses/${p.id}/check`);
      if (alive && checkTurn[p.id] === mine) checks[p.id] = answer;
    } catch (e) {
      // A later check (or a save's) has the say.
      if (!alive || checkTurn[p.id] !== mine) return;
      if (statusOf(e) === 503) return void (checks[p.id] = 'busy');
      delete checks[p.id];
      failed(e);
    }
  }
  /** A check's pass as the save answered it, which no earlier check may overwrite. */
  function passed(saved: Proxy & { latency_ms?: number }) {
    checkTurn[saved.id] = (checkTurn[saved.id] ?? 0) + 1;
    if (typeof saved.latency_ms === 'number') checks[saved.id] = { ok: true, latency_ms: saved.latency_ms };
    else void check(saved);
  }

  function open(proxy: Proxy | null) {
    menu = null;
    turn++;
    name = proxy?.name ?? '';
    address = '';
    timezone = proxy?.timezone ?? '';
    languages = proxy ? [...proxy.languages] : [];
    dialogError = '';
    saving = false;
    editing = { proxy };
    // Editing picks the time zone and languages from what the admin offers.
    if (proxy && !options) {
      const mine = turn;
      api<ProxyOptions>('GET', '/api/egresses/options').then(
        (o) => (options = o),
        (e: unknown) => {
          if (alive && mine === turn) failed(e, (text) => (dialogError = text));
        },
      );
    }
  }

  /** Closes the dialog; the address typed, which holds the proxy's password, goes with it. */
  function close() {
    turn++;
    editing = null;
    address = '';
    saving = false;
  }

  function addLanguage(tag: string) {
    if (tag && !languages.includes(tag)) languages = [...languages, tag];
  }

  const typed = $derived(address.trim());
  const canSave = $derived.by(() => !saving && name.trim() !== '' && (editing?.proxy ? languages.length > 0 : typed !== ''));

  /**
   * Connects a new proxy, or saves an edited one: the admin checks a new address first and saves only when it passes,
   * with where its exit is; a failed check comes back with its reason, and the dialog stays as it was.
   */
  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!canSave) return;
    const target = editing?.proxy ?? null;
    const mine = turn;
    const body: Record<string, unknown> = { name: name.trim() };
    if (typed !== '') body.proxy = typed;
    if (target) {
      body.timezone = timezone;
      body.languages = languages;
    }
    saving = true;
    dialogError = '';
    try {
      const saved = target
        ? await api<Proxy & { latency_ms?: number }>('PATCH', `/api/egresses/${target.id}`, body)
        : await api<Proxy & { latency_ms?: number }>('POST', '/api/egresses', body);
      if (!alive || mine !== turn) return;
      if (!target || typed !== '') passed(saved);
      close();
      if (!target) {
        toast = `“${saved.name}” is connected${saved.country_name ? ` from ${saved.country_name}` : ''}.`;
        clearTimeout(toastTimer);
        toastTimer = setTimeout(() => (toast = ''), 4000);
      }
      load();
    } catch (e) {
      if (!alive || mine !== turn) return;
      saving = false;
      // A check that failed comes back as a refusal with its reason; anything else says what it says (a lapsed login
      // goes to the login).
      const status = statusOf(e) ?? 0;
      const reason = bodyOf(e).reason;
      if (status !== 401 && status >= 400 && status < 500 && isCheckReason(reason)) dialogError = reasonText(reason);
      else if (status === 503) dialogError = `${BUSY}.`;
      else failed(e, (text) => (dialogError = text));
    }
  }

  async function remove(p: Proxy) {
    menu = null;
    if (!confirm(`Delete “${p.name}”? Sessions can no longer browse from it.`)) return;
    error = '';
    try {
      await api('DELETE', `/api/egresses/${p.id}`);
    } catch (e) {
      // The sessions that hold it, by their ids.
      const using = bodyOf(e).sessions;
      if (statusOf(e) === 409 && Array.isArray(using)) {
        error = `“${p.name}” is in use by ${using.length === 1 ? '1 session' : `${using.length} sessions`}. Close ${using.length === 1 ? 'it' : 'them'} first.`;
      } else failed(e);
    }
    load();
  }

  const PIN = PAGE_ICONS.proxies;
  const PLUG_ICON = 'M9 7V3M15 7V3M7 7h10v4a5 5 0 0 1-10 0zM12 16v5';
  const LANGUAGE_ICON = 'M4 5h9M8.5 3v2M6 5c.8 3.2 2.8 5.6 5.5 7M11 5c-.9 3.6-3.3 6.4-7 8M13 20l4-9 4 9M14.3 17h5.4';
  const ADDRESS_HELP = 'From your proxy provider. Supports http, https and socks5.';
</script>

{#snippet status(state: Check | 'checking' | 'busy' | null | undefined)}
  {#if state === 'checking'}
    <span class="{chip} {CHIP_TONES.neutral}"><span class="size-2.5 animate-spin rounded-full border-2 border-muted border-t-transparent" aria-hidden="true"></span>Checking…</span>
  {:else if state === 'busy'}
    <span class="{chip} {CHIP_TONES.neutral}">{BUSY}</span>
  {:else if state?.ok}
    <span class="{chip} {CHIP_TONES.good}"><span class="size-1.5 rounded-full bg-good" aria-hidden="true"></span>{['Connected', state.exit && exitText(state.exit), `${state.latency_ms} ms`].filter(Boolean).join(' · ')}</span>
  {:else if state}
    <span class="{chip} {CHIP_TONES.bad}"><span class="size-1.5 rounded-full bg-bad" aria-hidden="true"></span>{reasonText(state.reason)}</span>
  {/if}
{/snippet}

{#snippet plus()}
  <Icon d={PLUS_ICON} class="size-4" />
{/snippet}

<PageHeader icon={PIN} title="Proxies" subtitle="Make websites see your browser in another place, through a proxy you rent.">
  {#snippet action()}
    {#if proxies && proxies.length > 0}
      <button class="{primary} inline-flex items-center gap-2" type="button" onclick={() => open(null)}>{@render plus()}Add proxy</button>
    {/if}
  {/snippet}
</PageHeader>

<p class="mt-6 font-semibold text-bad empty:hidden" role="alert">{error}</p>

{#if proxies === null}
  {#if !error}<Skeleton said="Loading your proxies" />{/if}
{:else if proxies.length === 0}
  <div class="mt-20 text-center">
    <span class="mx-auto mb-4 grid size-14 place-items-center rounded-2xl bg-accent-soft text-accent"><Icon d={PIN} class="size-7" /></span>
    <p class="text-lg font-semibold">No proxies yet</p>
    <p class="mx-auto mt-1.5 max-w-sm text-sm text-muted">Make websites see your browser in another place, through a proxy you rent.</p>
    <button class="{primary} mt-6 inline-flex items-center gap-2" type="button" onclick={() => open(null)}>{@render plus()}Add proxy</button>
  </div>
{:else}
  <ul class="mt-10 {rows}">
    {#each proxies as p (p.id)}
      <li class="flex items-center gap-4 py-3 {row}">
        <span class={rowBadge}><Icon d={PIN} class="size-5" /></span>
        <span class="min-w-0 flex-1">
          <span class="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span class="truncate font-semibold">{p.name}</span>
            {@render status(checks[p.id])}
          </span>
          <span class="mt-1 flex flex-wrap items-center gap-1.5 text-sm text-muted">
            <span class="{chip} {CHIP_TONES.neutral}">{#if flagOf(p.country)}<span aria-hidden="true">{flagOf(p.country)}</span>{:else}<Icon d={FORM_ICONS.website} class="size-3" />{/if}{p.country_name ?? zoneName(p.timezone)}</span>
            <span class="font-mono text-[13px]">{addressText(p)}</span>
            {#if p.in_use > 0}<span class="{chip} {CHIP_TONES.accent}">In use by {p.in_use === 1 ? '1 session' : `${p.in_use} sessions`}</span>{/if}
          </span>
        </span>
        <Menu label="More for {p.name}" bind:open={() => menu === p.id, (on) => (menu = on ? p.id : null)}>
          <button class={menuItem} type="button" onclick={() => open(p)}>Edit</button>
          <button class={menuItem} type="button" onclick={() => check(p)}>Test</button>
          <button class="{menuItem} text-bad" type="button" onclick={() => remove(p)}>Delete</button>
        </Menu>
      </li>
    {/each}
  </ul>
{/if}

{#if toast}<p class="fixed bottom-5 left-1/2 z-40 -translate-x-1/2 rounded-full bg-ink px-4 py-2 text-sm font-semibold text-white shadow-lg" role="status">{toast}</p>{/if}

<Dialog
  title={editing?.proxy ? `Edit “${editing.proxy.name}”` : 'Add proxy'}
  icon={PIN}
  subtitle={editing?.proxy ? 'A new address is checked before it replaces the old one.' : 'Checked as it connects: websites see its browsers where it is.'}
  bind:open={() => editing !== null, (on) => !on && !saving && close()}
>
  <form onsubmit={save}>
    <Field icon={FORM_ICONS.name} label="Name" for="proxy-name">
      <input id="proxy-name" class={field} type="text" maxlength="32" pattern={NAME_PATTERN} title="Lowercase letters, digits and dashes" autocapitalize="none" spellcheck="false" placeholder="e.g. frankfurt" required bind:value={name} />
    </Field>
    <Field
      icon={FORM_ICONS.key}
      label="Proxy address"
      for="proxy-address"
      help={editing?.proxy
        ? `Now ${editing.proxy.scheme}://${addressText(editing.proxy)}${editing.proxy.has_user ? (editing.proxy.has_password ? ', with a user name and password' : ', with a user name') : ''}. Leave empty to keep it; a new address replaces its user name and password too.`
        : ADDRESS_HELP}
    >
      <input
        id="proxy-address"
        class="{field} font-mono text-sm"
        type="text"
        autocomplete="off"
        spellcheck="false"
        placeholder="http://user:password@host:port"
        required={!editing?.proxy}
        aria-describedby="proxy-address-help"
        bind:value={address}
      />
    </Field>
    {#if editing?.proxy}
      <Field icon={FORM_ICONS.website} label="Time zone" for="proxy-timezone" help="The browser’s clock, as websites see it.">
        <select id="proxy-timezone" class={field} aria-describedby="proxy-timezone-help" bind:value={timezone}>
          {#if !options?.timezones.includes(timezone)}<option value={timezone}>{zoneName(timezone)}</option>{/if}
          {#each options?.timezones ?? [] as z (z)}<option value={z}>{zoneName(z)}</option>{/each}
        </select>
      </Field>
      <div class="mt-5">
        <p class="mb-1.5 flex items-center gap-2 text-sm font-semibold" id="proxy-languages-label"><Icon d={LANGUAGE_ICON} class="size-[18px] shrink-0 text-accent" />Languages</p>
        <ul class="flex flex-wrap gap-1.5" aria-labelledby="proxy-languages-label">
          {#each languages as t (t)}
            <li class="{chip} {CHIP_TONES.accent} py-1 pr-1">
              {languageName(t, options?.languages)}
              <button
                class="grid size-4 place-items-center rounded-full hover:bg-accent/15 disabled:opacity-40"
                type="button"
                aria-label="Remove {languageName(t, options?.languages)}"
                disabled={languages.length === 1}
                onclick={() => (languages = languages.filter((x) => x !== t))}><Icon d="M7 7l10 10M17 7 7 17" class="size-3" /></button
              >
            </li>
          {/each}
        </ul>
        <select class="{field} mt-2" aria-label="Add a language" value="" onchange={(e) => (addLanguage(e.currentTarget.value), (e.currentTarget.value = ''))}>
          <option value="">Add a language…</option>
          {#each (options?.languages ?? []).filter((l) => !languages.includes(l.tag)) as l (l.tag)}<option value={l.tag}>{l.name}</option>{/each}
        </select>
        <p class="mt-1.5 text-[13px] text-muted">The ones websites are asked for, first preferred.</p>
      </div>
    {/if}
    {#if dialogError}
      <p class="mt-5 flex items-start gap-2.5 rounded-2xl bg-bad/[0.07] px-4 py-3 text-sm font-semibold text-bad" role="alert">
        <Icon d={GATE_ICONS.warning} class="mt-px size-4 shrink-0" />{dialogError}
      </p>
    {/if}
    <div class="mt-7 flex justify-end gap-3">
      <button class={button} type="button" disabled={saving} onclick={close}>Cancel</button>
      <button class="{primary} inline-flex items-center gap-2" type="submit" disabled={!canSave}>
        {#if saving}
          <span class="size-4 animate-spin rounded-full border-2 border-white border-t-transparent" aria-hidden="true"></span>{editing?.proxy ? 'Saving…' : 'Connecting…'}
        {:else if editing?.proxy}
          Save
        {:else}
          <Icon d={PLUG_ICON} class="size-4" />{dialogError ? 'Try again' : 'Connect'}
        {/if}
      </button>
    </div>
  </form>
</Dialog>
