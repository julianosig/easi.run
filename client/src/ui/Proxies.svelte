<script lang="ts">
  // The proxies (egresses, as the API calls them) a session may browse from: connect one (the admin checks it and saves
  // it with where its exit is, in one step); edit, test or delete one. Each is checked as the page opens, one after
  // another (the server runs only a few checks at once), so its row says whether it works now.
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { CHIP_TONES, chip, FORM_ICONS, menuItem, PAGE_ICONS, row, rowBadge, rows } from './admin.ts';
  import { api, bodyOf, statusOf } from './api.ts';
  import { requests } from './requests.svelte.ts';
  import Dialog from './Dialog.svelte';
  import Field from './Field.svelte';
  import EmptyState from './EmptyState.svelte';
  import Icon from './Icon.svelte';
  import NewButton from './NewButton.svelte';
  import Menu from './Menu.svelte';
  import PageHeader from './PageHeader.svelte';
  import { addressText, BUSY, type Check, countryName, exitText, flagOf, isCheckReason, languageName, NAME_PATTERN, type Proxy, type ProxyList, type ProxyOptions, reasonKey, zoneName } from './proxies.ts';
  import { tick } from 'svelte';
  import { button, field, GATE_ICONS, primary, textLink } from './ui.ts';
  import Skeleton from './Skeleton.svelte';

  let { onLogin }: { onLogin: () => void } = $props();

  let proxies: Proxy[] | null = $state(null);
  /** Each proxy's last check here, that one is under way, or that the server was too busy to run it. */
  let checks: Record<string, Check | 'checking' | 'busy'> = $state({});
  /** The latest check asked for each proxy: an earlier one's late answer changes nothing. */
  const checkTurn: Record<string, number> = {};
  let error: Said | null = $state(null);
  let menu: string | null = $state(null);
  /** The dialog: a new proxy (connected and saved in one step), or one being edited. */
  let editing: { proxy: Proxy | null } | null = $state(null);
  /** Bumped as the dialog opens and closes: an answer for an earlier one changes nothing. */
  let turn = 0;
  let name = $state('');
  let address = $state('');
  /** Editing: the URL field is open to replace the proxy's URL (closed, the URL it has now stays). */
  let changing = $state(false);
  /** An edited proxy's time zone and languages, picked from what the admin offers. */
  let timezone = $state('');
  let languages: string[] = $state([]);
  let options: ProxyOptions | null = $state(null);
  let saving = $state(false);
  /** Why the last connect or save failed: a check's reason in plain words, or the admin's message. */
  let dialogError: Said | null = $state(null);
  /** Said a moment after a proxy is connected. */
  let toast: Said | null = $state(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => () => clearTimeout(toastTimer));
  const page = requests(() => onLogin());
  const alive = page.alive;
  const failed = (e: unknown, where: (text: string) => void = (text) => (error = () => text)) => page.fail(e, where);

  async function load() {
    try {
      const list = (await api<ProxyList>('GET', '/api/egresses')).egresses;
      if (!alive()) return;
      proxies = list;
      // One at a time: all at once, most would find the server busy.
      for (const p of list) {
        if (!alive()) return;
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
      if (alive() && checkTurn[p.id] === mine) checks[p.id] = answer;
    } catch (e) {
      // A later check (or a save's) has the say.
      if (!alive() || checkTurn[p.id] !== mine) return;
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
    changing = false;
    timezone = proxy?.timezone ?? '';
    languages = proxy ? [...proxy.languages] : [];
    dialogError = null;
    saving = false;
    editing = { proxy };
    // Editing picks the time zone and languages from what the admin offers.
    if (proxy && !options) {
      const mine = turn;
      api<ProxyOptions>('GET', '/api/egresses/options').then(
        (o) => (options = o),
        (e: unknown) => {
          if (alive() && mine === turn) failed(e, (text) => (dialogError = () => text));
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
    dialogError = null;
    try {
      const saved = target
        ? await api<Proxy & { latency_ms?: number }>('PATCH', `/api/egresses/${target.id}`, body)
        : await api<Proxy & { latency_ms?: number }>('POST', '/api/egresses', body);
      if (!alive() || mine !== turn) return;
      if (!target || typed !== '') passed(saved);
      close();
      if (!target) {
        const { name, country: code, country_name: english } = saved;
        toast = () => {
          const country = countryName(code, english, i18n.locale);
          return country ? t('proxies.connected.from', { name, country }) : t('proxies.connected', { name });
        };
        clearTimeout(toastTimer);
        toastTimer = setTimeout(() => (toast = null), 4000);
      }
      load();
    } catch (e) {
      if (!alive() || mine !== turn) return;
      saving = false;
      // A check that failed comes back as a refusal with its reason; anything else says what it says (a lapsed login
      // goes to the login).
      const status = statusOf(e) ?? 0;
      const reason = bodyOf(e).reason;
      if (status !== 401 && status >= 400 && status < 500 && isCheckReason(reason)) dialogError = () => t(reasonKey(reason));
      else if (status === 503) dialogError = () => t('proxies.save.busy');
      else failed(e, (text) => (dialogError = () => text));
    }
  }

  async function remove(p: Proxy) {
    menu = null;
    if (!confirm(t('proxies.delete.confirm', { name: p.name }))) return;
    error = null;
    try {
      await api('DELETE', `/api/egresses/${p.id}`);
    } catch (e) {
      // The sessions that hold it, by their ids.
      const using = bodyOf(e).sessions;
      if (statusOf(e) === 409 && Array.isArray(using)) {
        error = () => t('proxies.delete.inUse', { count: using.length, name: p.name });
      } else failed(e);
    }
    load();
  }

  const PIN = PAGE_ICONS.proxies;
  const PLUG_ICON = 'M9 7V3M15 7V3M7 7h10v4a5 5 0 0 1-10 0zM12 16v5';
  const LANGUAGE_ICON = 'M4 5h9M8.5 3v2M6 5c.8 3.2 2.8 5.6 5.5 7M11 5c-.9 3.6-3.3 6.4-7 8M13 20l4-9 4 9M14.3 17h5.4';
</script>

{#snippet status(state: Check | 'checking' | 'busy' | null | undefined)}
  {#if state === 'checking'}
    <span class="{chip} {CHIP_TONES.neutral}"><span class="size-2.5 animate-spin rounded-full border-2 border-muted border-t-transparent" aria-hidden="true"></span>{t('proxies.check.checking')}</span>
  {:else if state === 'busy'}
    <span class="{chip} {CHIP_TONES.neutral}">{t(BUSY)}</span>
  {:else if state?.ok}
    <span class="{chip} {CHIP_TONES.good}"><span class="size-1.5 rounded-full bg-good" aria-hidden="true"></span>{state.exit ? t('proxies.check.passed.exit', { exit: exitText(state.exit, i18n.locale), latency: state.latency_ms }) : t('proxies.check.passed', { latency: state.latency_ms })}</span>
  {:else if state}
    <span class="{chip} {CHIP_TONES.bad}"><span class="size-1.5 rounded-full bg-bad" aria-hidden="true"></span>{t(reasonKey(state.reason))}</span>
  {/if}
{/snippet}

<PageHeader icon={PIN} title={t('proxies.title')} subtitle={t('proxies.subtitle')}>
  {#snippet action()}
    {#if proxies}
      <NewButton label={t('proxies.add')} onclick={() => open(null)} />
    {/if}
  {/snippet}
</PageHeader>

<p class="mt-6 font-semibold text-bad empty:hidden" role="alert">{error?.()}</p>

{#if proxies === null}
  {#if !error}<Skeleton said={t('proxies.loading')} />{/if}
{:else if proxies.length === 0}
  <EmptyState
    icon={PIN}
    title={t('proxies.empty.title')}
    text={t('proxies.empty.text')}
    points={[t('proxies.empty.checked'), t('proxies.empty.around')]}
  />
{:else}
  <ul class="mt-10 {rows}">
    {#each proxies as p (p.id)}
      {@const state = checks[p.id]}
      <li class="flex items-center gap-4 py-3 {row}">
        <span class={rowBadge}><Icon d={PIN} class="size-5" /></span>
        <span class="min-w-0 flex-1">
          <span class="block truncate font-semibold">{p.name}</span>
          <span class="mt-1 flex flex-wrap items-center gap-1.5 text-sm text-muted">
            {@render status(state)}
            <!-- A check that found the exit says where it is and its address already. -->
            {#if !(state && typeof state === 'object' && state.ok && state.exit)}
              <span class="{chip} {CHIP_TONES.neutral}">{#if flagOf(p.country)}<span aria-hidden="true">{flagOf(p.country)}</span>{:else}<Icon d={FORM_ICONS.website} class="size-3" />{/if}{countryName(p.country, p.country_name, i18n.locale) ?? zoneName(p.timezone)}</span>
              <span class="font-mono text-[13px]">{addressText(p)}</span>
            {/if}
            {#if p.in_use > 0}<span class="{chip} {CHIP_TONES.accent}">{t('proxies.proxy.inUse', { count: p.in_use })}</span>{/if}
          </span>
        </span>
        <Menu label={t('proxies.proxy.more', { name: p.name })} bind:open={() => menu === p.id, (on) => (menu = on ? p.id : null)}>
          <button class={menuItem} type="button" onclick={() => open(p)}>{t('proxies.proxy.edit')}</button>
          <button class={menuItem} type="button" onclick={() => check(p)}>{t('proxies.proxy.test')}</button>
          <button class="{menuItem} text-bad" type="button" onclick={() => remove(p)}>{t('proxies.proxy.delete')}</button>
        </Menu>
      </li>
    {/each}
  </ul>
{/if}

{#if toast}<p class="fixed bottom-5 left-1/2 z-40 -translate-x-1/2 rounded-full bg-ink px-4 py-2 text-sm font-semibold text-white shadow-lg" role="status">{toast()}</p>{/if}

<Dialog
  title={editing?.proxy ? t('proxies.edit.title', { name: editing.proxy.name }) : t('proxies.add.title')}
  icon={PIN}
  subtitle={editing?.proxy ? t('proxies.edit.subtitle') : t('proxies.add.subtitle')}
  bind:open={() => editing !== null, (on) => !on && !saving && close()}
>
  <form onsubmit={save}>
    <Field icon={FORM_ICONS.name} label={t('proxies.name')} for="proxy-name">
      <input id="proxy-name" class={field} type="text" maxlength="32" pattern={NAME_PATTERN} title={t('proxies.name.rule')} autocapitalize="none" spellcheck="false" placeholder={t('proxies.name.placeholder')} required bind:value={name} />
    </Field>
    {#if editing?.proxy && !changing}
      {@const now = `${editing.proxy.scheme}://${addressText(editing.proxy)}`}
      <!-- The URL it has now, kept unless the person changes it. -->
      <div class="mt-5">
        <p class="mb-1.5 flex items-center gap-2 text-sm font-semibold"><Icon d={FORM_ICONS.key} class="size-[18px] shrink-0 text-accent" />{t('proxies.address')}</p>
        <p class="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-sm text-muted">
          <span>{t(editing.proxy.has_user ? (editing.proxy.has_password ? 'proxies.address.now.password' : 'proxies.address.now.user') : 'proxies.address.now', { address: now })}</span>
          <button class={textLink} type="button" onclick={() => ((changing = true), tick().then(() => document.getElementById('proxy-address')?.focus()))}>{t('proxies.address.change')}</button>
        </p>
      </div>
    {:else}
    <Field icon={FORM_ICONS.key} label={t('proxies.address')} for="proxy-address" help={editing?.proxy ? t('proxies.address.replaces') : t('proxies.address.help')}>
      <input
        id="proxy-address"
        class="{field} font-mono text-sm"
        type="text"
        autocomplete="off"
        spellcheck="false"
        placeholder={t('proxies.address.placeholder')}
        required={!editing?.proxy}
        aria-describedby="proxy-address-help"
        bind:value={address}
      />
    </Field>
    {/if}
    {#if editing?.proxy}
      <Field icon={FORM_ICONS.website} label={t('proxies.timezone')} for="proxy-timezone" help={t('proxies.timezone.help')}>
        <select id="proxy-timezone" class={field} aria-describedby="proxy-timezone-help" bind:value={timezone}>
          {#if !options?.timezones.includes(timezone)}<option value={timezone}>{zoneName(timezone)}</option>{/if}
          {#each options?.timezones ?? [] as z (z)}<option value={z}>{zoneName(z)}</option>{/each}
        </select>
      </Field>
      <div class="mt-5">
        <p class="mb-1.5 flex items-center gap-2 text-sm font-semibold" id="proxy-languages-label"><Icon d={LANGUAGE_ICON} class="size-[18px] shrink-0 text-accent" />{t('proxies.languages')}</p>
        <ul class="flex flex-wrap gap-1.5" aria-labelledby="proxy-languages-label">
          {#each languages as tag (tag)}
            <li class="{chip} {CHIP_TONES.accent} py-1 pr-1">
              {languageName(tag, i18n.locale, options?.languages)}
              <button
                class="grid size-4 place-items-center rounded-full hover:bg-accent/15 disabled:opacity-40"
                type="button"
                aria-label={t('proxies.languages.remove', { language: languageName(tag, i18n.locale, options?.languages) })}
                disabled={languages.length === 1}
                onclick={() => (languages = languages.filter((x) => x !== tag))}><Icon d="M7 7l10 10M17 7 7 17" class="size-3" /></button
              >
            </li>
          {/each}
        </ul>
        <select class="{field} mt-2" aria-label={t('proxies.languages.add')} value="" onchange={(e) => (addLanguage(e.currentTarget.value), (e.currentTarget.value = ''))}>
          <option value="">{t('proxies.languages.add.prompt')}</option>
          {#each (options?.languages ?? []).filter((l) => !languages.includes(l.tag)) as l (l.tag)}<option value={l.tag}>{languageName(l.tag, i18n.locale, options?.languages)}</option>{/each}
        </select>
        <p class="mt-1.5 text-[13px] text-muted">{t('proxies.languages.help')}</p>
      </div>
    {/if}
    {#if dialogError}
      <p class="mt-5 flex items-start gap-2.5 rounded-2xl bg-bad/[0.07] px-4 py-3 text-sm font-semibold text-bad" role="alert">
        <Icon d={GATE_ICONS.warning} class="mt-px size-4 shrink-0" />{dialogError()}
      </p>
    {/if}
    <div class="mt-7 flex justify-end gap-3">
      <button class={button} type="button" disabled={saving} onclick={close}>{t('proxies.cancel')}</button>
      <button class="{primary} inline-flex items-center gap-2" type="submit" disabled={!canSave}>
        {#if saving}
          <span class="size-4 animate-spin rounded-full border-2 border-white border-t-transparent" aria-hidden="true"></span>{editing?.proxy ? t('proxies.saving') : t('proxies.connecting')}
        {:else if editing?.proxy}
          {t('proxies.save')}
        {:else}
          <Icon d={PLUG_ICON} class="size-4" />{dialogError ? t('proxies.retry') : t('proxies.connect')}
        {/if}
      </button>
    </div>
  </form>
</Dialog>
