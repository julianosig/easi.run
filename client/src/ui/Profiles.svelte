<script lang="ts">
  // The saved profiles: create one, rename it, choose the agents that may start sessions from it, delete it; and, after a
  // sign-in that checked no password (a passkey's), the password once to unlock them. A session started from a profile
  // has its logins, and saves them when it is stopped.
  import { api, messageOf, statusOf, type TokenInfo } from './api.ts';
  import { allowedNames, LOCAL, type ProfileInfo, type ProfileList, profileSummary, toggled } from './profiles.ts';
  import { button, card, field, label, panelTitle, primary } from './ui.ts';

  let { onChange, onLogin }: { onChange: () => void; onLogin: () => void } = $props();

  let list: ProfileList | null = $state(null);
  let tokens: TokenInfo[] = $state([]);
  let name = $state('');
  let password = $state('');
  /** The profile being renamed, and the name typed for it. */
  let renaming: string | null = $state(null);
  let newName = $state('');
  /** The profile whose agents are shown. */
  let choosing: string | null = $state(null);
  let busy = $state(false);
  let error = $state('');

  async function load() {
    try {
      list = await api<ProfileList>('GET', '/api/profiles');
      tokens = (await api<{ tokens: TokenInfo[] }>('GET', '/api/tokens')).tokens;
    } catch (e) {
      if (statusOf(e) === 401) return onLogin();
      error = messageOf(e);
    }
  }
  $effect(() => {
    load();
  });

  /** Runs one change, then shows the list as it is now. */
  async function change(run: () => Promise<unknown>) {
    busy = true;
    error = '';
    try {
      await run();
      onChange();
    } catch (e) {
      if (statusOf(e) === 401) return onLogin();
      error = messageOf(e);
    }
    busy = false;
    load();
  }

  function create(event: SubmitEvent) {
    event.preventDefault();
    change(async () => {
      await api('POST', '/api/profiles', { name });
      name = '';
    });
  }

  function unlock(event: SubmitEvent) {
    event.preventDefault();
    change(async () => {
      try {
        await api('POST', '/api/profiles/unlock', { password });
        password = '';
      } catch (e) {
        if (statusOf(e) === 403) throw new Error('That is not your current password.');
        if (statusOf(e) === 429) throw new Error('Too many attempts. Wait a minute, then try again.');
        throw e;
      }
    });
  }

  function rename(event: SubmitEvent) {
    event.preventDefault();
    const id = renaming;
    if (id === null) return;
    change(async () => {
      await api('PUT', `/api/profiles/${id}`, { name: newName });
      renaming = null;
    });
  }

  function allow(profile: ProfileInfo, entry: string, on: boolean) {
    change(() => api('PUT', `/api/profiles/${profile.id}`, { allowed: toggled(profile.allowed, entry, on) }));
  }

  function remove(profile: ProfileInfo) {
    if (!confirm(`Delete “${profile.name}”? Its saved logins and site data are deleted for good.`)) return;
    change(() => api('DELETE', `/api/profiles/${profile.id}`));
  }

  const when = (seconds: number) => new Date(seconds * 1000).toLocaleString();
  const agents = (profile: ProfileInfo) => {
    const names = allowedNames(profile, tokens);
    return names.length === 0 ? 'Only you' : `You and ${names.join(', ')}`;
  };
</script>

<section class="{card} lg:col-span-2">
  <h2 class={panelTitle}>Saved profiles</h2>
  <p class="text-sm text-muted">
    A session started from a saved profile keeps your logins and site data, and saves them when you stop it. Each is
    encrypted on the server with a key only your password unlocks. One session at a time uses a profile; agents use
    the ones you allow them.
  </p>
  {#if list && !list.unlocked}
    <form class="mt-4 rounded-xl border border-line bg-wash p-4" onsubmit={unlock}>
      <p class="font-semibold">Unlock saved profiles</p>
      <p class="text-sm text-muted">A passkey sign-in does not unlock them. Enter your password once; they stay unlocked until the server restarts.</p>
      <label class={label} for="unlock-password">Your password</label>
      <div class="flex flex-wrap items-end gap-3">
        <input id="unlock-password" class="{field} min-w-48 flex-1" type="password" autocomplete="current-password" required bind:value={password} />
        <button class={primary} type="submit" disabled={busy}>Unlock</button>
      </div>
    </form>
  {/if}
  {#if list && list.unlocked && list.profiles.length < list.max}
    <form class="mt-2 flex flex-wrap items-end gap-3" onsubmit={create}>
      <span class="min-w-48 flex-1">
        <label class={label} for="profile-name">Name</label>
        <input id="profile-name" class={field} type="text" maxlength="64" placeholder="Personal" required bind:value={name} />
      </span>
      <button class={primary} type="submit" disabled={busy}>Create profile</button>
    </form>
  {/if}
  <p class="mt-3 font-semibold text-bad" role="alert">{error}</p>
  {#if list === null}
    <p class="text-muted">Loading…</p>
  {:else if list.profiles.length === 0}
    <p class="text-muted">No saved profiles.</p>
  {:else}
    <ul class="divide-y divide-line">
      {#each list.profiles as p (p.id)}
        <li class="py-3">
          <div class="flex flex-wrap items-center gap-3">
            {#if renaming === p.id}
              <form class="flex min-w-0 flex-1 flex-wrap items-center gap-3" onsubmit={rename}>
                <input class="{field} min-w-48 flex-1" type="text" maxlength="64" required aria-label="New name" bind:value={newName} />
                <button class={primary} type="submit" disabled={busy}>Save</button>
                <button class={button} type="button" onclick={() => (renaming = null)}>Cancel</button>
              </form>
            {:else}
              <span class="min-w-0 flex-1">
                <span class="block truncate font-semibold">{p.name}</span>
                <span class="text-sm text-muted">{profileSummary(p, when)} · {agents(p)}</span>
                {#if p.unsaved}<span class="block text-sm font-semibold text-bad">Last save failed: {p.unsaved}</span>{/if}
              </span>
              <button class={button} type="button" onclick={() => ((renaming = p.id), (newName = p.name))}>Rename</button>
              <button class={button} type="button" aria-expanded={choosing === p.id} onclick={() => (choosing = choosing === p.id ? null : p.id)}>Agents</button>
              <button class={button} type="button" onclick={() => remove(p)} disabled={p.session !== null}>Delete</button>
            {/if}
          </div>
          {#if choosing === p.id}
            <fieldset class="mt-3 rounded-xl border border-line bg-wash p-4">
              <legend class="px-1 text-sm font-semibold">Agents that may start sessions from “{p.name}”</legend>
              <label class="mt-1 flex items-center gap-2.5 text-sm">
                <input class="accent-accent" type="checkbox" checked={p.allowed.includes(LOCAL)} disabled={busy} onchange={(e) => allow(p, LOCAL, e.currentTarget.checked)} />
                Local MCP clients (<code>admin mcp</code>)
              </label>
              {#each tokens.filter((t) => !t.revoked) as t (t.id)}
                <label class="mt-1 flex items-center gap-2.5 text-sm">
                  <input class="accent-accent" type="checkbox" checked={p.allowed.includes(t.id)} disabled={busy} onchange={(e) => allow(p, t.id, e.currentTarget.checked)} />
                  {t.name}
                </label>
              {/each}
              {#if tokens.length === 0}<p class="mt-1 text-sm text-muted">No agent tokens yet; create one below.</p>{/if}
            </fieldset>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>
