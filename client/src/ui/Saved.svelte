<script lang="ts">
  // The saved sessions: persistent sessions not running now (each a saved profile, as the API calls them), with their
  // logins and site data. Resume one, rename it, choose the agents that may use it, delete it; and, after a sign-in
  // that checked no password (a passkey's), the password once to unlock them.
  import Button from './Button.svelte';
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { CHIP_TONES, chip, row, rowBadge, rows } from './admin.ts';
  import Icon from './Icon.svelte';
  import { api, statusOf, type TokenInfo } from './api.ts';
  import { saidOf } from './errors.ts';
  import { requests } from './requests.svelte.ts';
  import Dialog from './Dialog.svelte';
  import Field from './Field.svelte';
  import { LOCAL, type ProfileInfo, type ProfileList, profileSummary, toggled } from './profiles.ts';
  import Switch from './Switch.svelte';
  import { field, textLink } from './ui.ts';

  let {
    list,
    tokens,
    onChange,
    onResume,
    onLogin,
    onAgents,
    listed = true,
    hint = true,
  }: {
    list: ProfileList;
    /** The agents' tokens; null until they are read (then nothing who may start one can be saved). */
    tokens: TokenInfo[] | null;
    /** Something changed: the page lists them again. */
    onChange: () => void;
    onResume: (saved: ProfileInfo) => void;
    onLogin: () => void;
    /** Opens the agent tokens' page. */
    onAgents: () => void;
    /** Whether its rows show (the page's filter), and the hint on making one when there are none. */
    listed?: boolean;
    hint?: boolean;
  } = $props();

  const saved = $derived(list.profiles.filter((p) => p.session === null));
  let password = $state('');
  let unlocking = $state(false);
  /** The saved session being edited, and its name and agents as the dialog has them until saved. */
  let editing: ProfileInfo | null = $state(null);
  let newName = $state('');
  let allowedDraft: string[] = $state([]);
  let busy = $state(false);
  let error: Said | null = $state(null);
  /** What went wrong in a dialog. */
  let dialogError: Said | null = $state(null);

  const page = requests(() => onLogin());
  /** Runs one change, then has the page list them as they are now; `where` says what went wrong. */
  async function change(run: () => Promise<unknown>, where: (said: Said) => void = (said) => (error = said)): Promise<boolean> {
    busy = true;
    error = null;
    dialogError = null;
    let ok = false;
    try {
      await run();
      ok = true;
    } catch (e) {
      // Nothing once the page is gone; the login when it lapsed.
      page.fail(e, where);
      if (statusOf(e) === 401 || !page.alive()) return false;
    }
    busy = false;
    onChange();
    return ok;
  }

  async function unlock(event: SubmitEvent) {
    event.preventDefault();
    const ok = await change(
      async () => {
        await api('POST', '/api/profiles/unlock', { password });
        password = '';
      },
      (said) => (dialogError = said),
    );
    if (ok) unlocking = false;
  }

  function edit(entry: ProfileInfo) {
    dialogError = null;
    newName = entry.name;
    allowedDraft = [...entry.allowed];
    editing = entry;
  }

  /** Saves the name and the agents together. */
  async function save(event: SubmitEvent) {
    event.preventDefault();
    const target = editing;
    if (target === null || tokens === null) return;
    const known = tokens;
    // Only who may still be allowed: a token revoked meanwhile is no longer one.
    const allowed = allowedDraft.filter((id) => id === LOCAL || known.some((token) => token.id === id && !token.revoked));
    const ok = await change(() => api('PUT', `/api/profiles/${target.id}`, { name: newName, allowed }), (said) => (dialogError = said));
    if (ok) editing = null;
  }

  function remove(entry: ProfileInfo) {
    if (!confirm(t('sessions.delete.confirm', { name: entry.name }))) return;
    change(() => api('DELETE', `/api/profiles/${entry.id}`));
  }

  const when = (seconds: number) => new Date(seconds * 1000).toLocaleString(i18n.locale);
</script>

<section class="mt-1" aria-label={t('sessions.saved')}>
  <p class="mt-6 font-semibold text-bad empty:hidden" role="alert">{error?.()}</p>
  {#if !listed}
    <!-- Filtered out: only its dialogs stay. -->
  {:else if !list.unlocked}
    <div class="mt-4 flex flex-wrap items-center justify-between gap-4 rounded-2xl bg-wash px-5 py-4">
      <p class="max-w-lg text-sm text-muted">
        <span class="flex items-center gap-2 font-semibold text-ink"><Icon d={ICONS.lock} class="size-4" />{t('sessions.locked', { count: list.profiles.length })}</span>
        {t('sessions.locked.text')}
      </p>
      <Button variant="primary" icon={ICONS.unlock} onclick={() => ((dialogError = null), (unlocking = true))}>{t('sessions.unlock')}</Button>
    </div>
  {:else if saved.length === 0}
    {#if hint && list.profiles.length === 0}<p class="mt-4 text-sm text-muted">{t('sessions.saved.none')}</p>{/if}
  {:else}
    <ul class={rows}>
      {#each saved as p (p.id)}
        <li class="flex items-center gap-4 py-3 {row}">
          <span class="{rowBadge} bg-wash text-muted"><Icon d={ICONS.persistent} class="size-5" /></span>
          <span class="min-w-0 flex-1">
            <span class="flex flex-wrap items-center gap-x-2 gap-y-1">
              <span class="truncate font-semibold">{p.name}</span>
              <span class="{chip} {CHIP_TONES.neutral}">{t('sessions.chip.saved')}</span>
            </span>
            <span class="mt-1 block truncate text-sm text-muted">{profileSummary(p, when, t, i18n.locale)}</span>
            {#if p.unsaved}<span class="mt-1 block text-sm font-semibold text-bad">{saidOf(p.unsaved)()}</span>{/if}
          </span>
          <Button size="sm" icon={ICONS.start} onclick={() => onResume(p)}>{t('sessions.resume')}</Button>
          <span class="flex w-[4.75rem] shrink-0 justify-end gap-1">
            <Button variant="ghost" tone="accent" size="sm" icon={ICONS.edit} label={t('sessions.edit', { name: p.name })} onclick={() => edit(p)} />
            <Button variant="ghost" tone="danger" size="sm" icon={ICONS.delete} label={t('sessions.delete', { name: p.name })} onclick={() => remove(p)} />
          </span>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<Dialog title={t('sessions.unlock.title')} icon={ICONS.lock} subtitle={t('sessions.unlock.subtitle')} bind:open={() => unlocking, (on) => busy || (unlocking = on)}>
  <form onsubmit={unlock}>
    <Field icon={ICONS.password} label={t('sessions.unlock.password')} for="unlock-password" help={t('sessions.unlock.password.help')}>
      <input id="unlock-password" class={field} type="password" autocomplete="current-password" placeholder={t('sessions.unlock.password')} required aria-describedby="unlock-password-help" bind:value={password} />
    </Field>
    <p class="mt-4 text-sm font-semibold text-bad empty:hidden" role="alert">{dialogError?.()}</p>
    <div class="mt-7 flex justify-end gap-3">
      <Button disabled={busy} onclick={() => (unlocking = false)}>{t('sessions.cancel')}</Button>
      <Button variant="primary" icon={ICONS.unlock} type="submit" disabled={busy}>{t('sessions.unlock')}</Button>
    </div>
  </form>
</Dialog>

<Dialog title={t('sessions.edit.title', { name: editing?.name ?? '' })} icon={ICONS.edit} subtitle={t('sessions.edit.subtitle')} bind:open={() => editing !== null, (on) => !on && !busy && (editing = null)}>
  <form onsubmit={save}>
    <Field icon={ICONS.name} label={t('sessions.name')} for="rename" help={t('sessions.name.help')}>
      <input id="rename" class={field} type="text" placeholder={t('sessions.name.placeholder')} required aria-describedby="rename-help" disabled={busy} bind:value={newName} />
    </Field>
    <p class="mt-6 flex items-center gap-2 text-sm font-semibold"><Icon d={ICONS.agents} class="size-[18px] shrink-0 text-accent" />{t('sessions.agents')}</p>
    <p class="mt-0.5 text-[13px] text-muted">{t('sessions.agents.subtitle')}</p>
    <div class="mt-2 -mx-3 space-y-0.5">
      <Switch
        icon={ICONS.terminal}
        title={t('sessions.agents.local')}
        help={t('sessions.agents.local.help')}
        checked={allowedDraft.includes(LOCAL)}
        disabled={busy}
        onchange={(on) => (allowedDraft = toggled(allowedDraft, LOCAL, on))}
      />
      {#each (tokens ?? []).filter((token) => !token.revoked) as token (token.id)}
        <Switch icon={ICONS.token} title={token.name} help={t('sessions.agents.token.help')} checked={allowedDraft.includes(token.id)} disabled={busy} onchange={(on) => (allowedDraft = toggled(allowedDraft, token.id, on))} />
      {/each}
    </div>
    {#if tokens !== null && tokens.length === 0}
      <p class="mt-3 text-sm text-muted">{t('sessions.agents.none.before')}<button class={textLink} type="button" onclick={onAgents}>{t('sessions.agents.none.link')}</button>{t('sessions.agents.none.after')}</p>
    {/if}
    <p class="mt-4 text-sm font-semibold text-bad empty:hidden" role="alert">{dialogError?.()}</p>
    <div class="mt-7 flex justify-end gap-3">
      <Button disabled={busy} onclick={() => (editing = null)}>{t('sessions.cancel')}</Button>
      <Button variant="primary" icon={ICONS.check} type="submit" disabled={busy || tokens === null}>{t('sessions.save')}</Button>
    </div>
  </form>
</Dialog>
