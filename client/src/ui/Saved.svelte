<script lang="ts">
  // The saved sessions: persistent sessions not running now (each a saved profile, as the API calls them), with their
  // logins and site data. Resume one, rename it, choose the agents that may use it, delete it; and, after a sign-in
  // that checked no password (a passkey's), the password once to unlock them.
  import { CHIP_TONES, chip, FORM_ICONS, groupLabel, menuItem, PAGE_ICONS, row, rowAction, rowBadge, rows } from './admin.ts';
  import Icon from './Icon.svelte';
  import { api, messageOf, NOT_YOUR_PASSWORD, statusOf, TOO_MANY_ATTEMPTS, type TokenInfo } from './api.ts';
  import Dialog from './Dialog.svelte';
  import Field from './Field.svelte';
  import Menu from './Menu.svelte';
  import { allowedNames, LOCAL, type ProfileInfo, type ProfileList, profileSummary, toggled } from './profiles.ts';
  import Switch from './Switch.svelte';
  import { button, field, primary } from './ui.ts';

  let {
    list,
    tokens,
    onChange,
    onResume,
    onLogin,
    onAgents,
  }: {
    list: ProfileList;
    tokens: TokenInfo[];
    /** Something changed: the page lists them again. */
    onChange: () => void;
    onResume: (saved: ProfileInfo) => void;
    onLogin: () => void;
    /** Opens the agent tokens' page. */
    onAgents: () => void;
  } = $props();

  const saved = $derived(list.profiles.filter((p) => p.session === null));
  let password = $state('');
  let unlocking = $state(false);
  /** The saved session being renamed, and the name typed for it. */
  let renaming: ProfileInfo | null = $state(null);
  let newName = $state('');
  /** The saved session whose agents are shown (by id: the list is read again after each change). */
  let choosing: string | null = $state(null);
  const chosen = $derived(list.profiles.find((p) => p.id === choosing) ?? null);
  // Its dialog stays open, not opened again, as each change lists them anew.
  const choosingOpen = $derived(chosen !== null);
  /** The row whose menu is open. */
  let menu: string | null = $state(null);
  let busy = $state(false);
  let error = $state('');
  /** What went wrong in a dialog. */
  let dialogError = $state('');

  /** Runs one change, then has the page list them as they are now; `where` says what went wrong. */
  async function change(run: () => Promise<unknown>, where: (text: string) => void = (text) => (error = text)): Promise<boolean> {
    busy = true;
    error = '';
    dialogError = '';
    let ok = false;
    try {
      await run();
      ok = true;
    } catch (e) {
      if (statusOf(e) === 401) return (onLogin(), false);
      where(messageOf(e));
    }
    busy = false;
    onChange();
    return ok;
  }

  async function unlock(event: SubmitEvent) {
    event.preventDefault();
    const ok = await change(
      async () => {
        try {
          await api('POST', '/api/profiles/unlock', { password });
          password = '';
        } catch (e) {
          if (statusOf(e) === 403) throw new Error(NOT_YOUR_PASSWORD);
          if (statusOf(e) === 429) throw new Error(TOO_MANY_ATTEMPTS);
          throw e;
        }
      },
      (text) => (dialogError = text),
    );
    if (ok) unlocking = false;
  }

  async function rename(event: SubmitEvent) {
    event.preventDefault();
    const target = renaming;
    if (target === null) return;
    const ok = await change(() => api('PUT', `/api/profiles/${target.id}`, { name: newName }), (text) => (dialogError = text));
    if (ok) renaming = null;
  }

  function allow(entry: ProfileInfo, who: string, on: boolean) {
    change(() => api('PUT', `/api/profiles/${entry.id}`, { allowed: toggled(entry.allowed, who, on) }), (text) => (dialogError = text));
  }

  function remove(entry: ProfileInfo) {
    menu = null;
    if (!confirm(`Delete “${entry.name}”? Its saved logins and site data are deleted for good.`)) return;
    change(() => api('DELETE', `/api/profiles/${entry.id}`));
  }

  const when = (seconds: number) => new Date(seconds * 1000).toLocaleString();
  const agents = (entry: ProfileInfo) => {
    const names = allowedNames(entry, tokens);
    return names.length === 0 ? 'Only you' : `You and ${names.join(', ')}`;
  };
</script>

<section class="mt-14" aria-labelledby="saved-title">
  <h2 id="saved-title" class={groupLabel}>Saved</h2>
  <p class="mt-6 font-semibold text-bad empty:hidden" role="alert">{error}</p>
  {#if !list.unlocked}
    <div class="mt-3 flex flex-wrap items-center justify-between gap-4 rounded-2xl bg-you/[0.07] px-5 py-4">
      <p class="max-w-lg text-sm text-muted">
        <span class="flex items-center gap-2 font-semibold text-ink"><Icon d={FORM_ICONS.lock} class="size-4" />{list.profiles.length === 1 ? '1 saved session is locked' : 'Your saved sessions are locked'}</span>
        A passkey sign-in does not unlock them. Enter your password once; they stay unlocked until the server restarts.
      </p>
      <button class="{primary} inline-flex items-center gap-2" type="button" onclick={() => ((dialogError = ''), (unlocking = true))}><Icon d={FORM_ICONS.key} class="size-4" />Unlock</button>
    </div>
  {:else if saved.length === 0}
    <p class="mt-3 text-sm text-muted">None. Turn on Persistent when you start a session to keep its logins for next time.</p>
  {:else}
    <ul class="mt-3 {rows}">
      {#each saved as p (p.id)}
        <li class="flex items-center gap-4 py-3 {row}">
          <span class="{rowBadge} bg-wash text-muted"><Icon d={FORM_ICONS.persistent} class="size-5" /></span>
          <span class="min-w-0 flex-1">
            <span class="flex flex-wrap items-center gap-x-2 gap-y-1">
              <span class="truncate font-semibold">{p.name}</span>
              <span class="{chip} {CHIP_TONES.neutral}">Saved</span>
            </span>
            <span class="mt-1 flex flex-wrap items-center gap-1.5 text-sm text-muted">
              <span>{profileSummary(p, when)}</span>
              <span class="{chip} {CHIP_TONES.neutral}"><Icon d={PAGE_ICONS.agents} class="size-3" />{agents(p)}</span>
            </span>
            {#if p.unsaved}<span class="mt-1 block text-sm font-semibold text-bad">Last save failed: {p.unsaved}</span>{/if}
          </span>
          <button class={rowAction} type="button" onclick={() => onResume(p)}><Icon d={FORM_ICONS.start} class="size-4 text-accent" />Resume</button>
          <Menu label="More for {p.name}" bind:open={() => menu === p.id, (on) => (menu = on ? p.id : null)}>
            <button class={menuItem} type="button" onclick={() => ((menu = null), (dialogError = ''), (newName = p.name), (renaming = p))}>Rename</button>
            <button class={menuItem} type="button" onclick={() => ((menu = null), (dialogError = ''), (choosing = p.id))}>Agents</button>
            <button class="{menuItem} text-bad" type="button" onclick={() => remove(p)}>Delete</button>
          </Menu>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<Dialog title="Unlock saved sessions" icon={FORM_ICONS.lock} subtitle="Once after each server restart, when you signed in with a passkey." bind:open={unlocking}>
  <form onsubmit={unlock}>
    <Field icon={FORM_ICONS.key} label="Your password" for="unlock-password" help="Opens the key their saved data is encrypted with.">
      <input id="unlock-password" class={field} type="password" autocomplete="current-password" required aria-describedby="unlock-password-help" bind:value={password} />
    </Field>
    <p class="mt-4 text-sm font-semibold text-bad empty:hidden" role="alert">{dialogError}</p>
    <div class="mt-7 flex justify-end gap-3">
      <button class={button} type="button" onclick={() => (unlocking = false)}>Cancel</button>
      <button class={primary} type="submit" disabled={busy}>Unlock</button>
    </div>
  </form>
</Dialog>

<Dialog title="Rename" icon={FORM_ICONS.rename} subtitle="Its logins and settings stay as they are." bind:open={() => renaming !== null, (on) => !on && (renaming = null)}>
  <form onsubmit={rename}>
    <Field icon={FORM_ICONS.name} label="Name" for="rename">
      <input id="rename" class={field} type="text" maxlength="64" required bind:value={newName} />
    </Field>
    <p class="mt-4 text-sm font-semibold text-bad empty:hidden" role="alert">{dialogError}</p>
    <div class="mt-7 flex justify-end gap-3">
      <button class={button} type="button" onclick={() => (renaming = null)}>Cancel</button>
      <button class={primary} type="submit" disabled={busy}>Save</button>
    </div>
  </form>
</Dialog>

<Dialog title="Agents" icon={PAGE_ICONS.agents} subtitle={chosen ? `Which agents may start “${chosen.name}”, besides you.` : ''} bind:open={() => choosingOpen, (on) => !on && (choosing = null)}>
  {#if chosen}
    <div class="mt-5 -mx-3 space-y-0.5">
      <Switch
        icon={FORM_ICONS.terminal}
        title="Agents on this server"
        help="Local MCP clients, through admin mcp."
        checked={chosen.allowed.includes(LOCAL)}
        disabled={busy}
        onchange={(on) => allow(chosen, LOCAL, on)}
      />
      {#each tokens.filter((t) => !t.revoked) as t (t.id)}
        <Switch icon={PAGE_ICONS.agents} title={t.name} help="With its agent token." checked={chosen.allowed.includes(t.id)} disabled={busy} onchange={(on) => allow(chosen, t.id, on)} />
      {/each}
    </div>
    {#if tokens.length === 0}
      <p class="mt-3 text-sm text-muted">No agent tokens yet; <button class="font-semibold text-accent hover:underline" type="button" onclick={onAgents}>create one in Agents</button>.</p>
    {/if}
    <p class="mt-4 text-sm font-semibold text-bad empty:hidden" role="alert">{dialogError}</p>
    <div class="mt-7 flex justify-end">
      <button class={primary} type="button" onclick={() => (choosing = null)}>Done</button>
    </div>
  {/if}
</Dialog>
