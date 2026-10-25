<script lang="ts">
  // The administrator's sessions: start one, from a saved profile or not, watch or close the running ones (an agent
  // token's too); the saved profiles; and the tokens.
  import { api, type Egress, messageOf, type SessionInfo, statusOf } from './api.ts';
  import { choiceLabel, type ProfileList } from './profiles.ts';
  import Profiles from './Profiles.svelte';
  import Tokens from './Tokens.svelte';
  import { button, card, field, label, panelTitle, primary } from './ui.ts';

  let { onWatch, onLogin }: { onWatch: (id: string) => void; onLogin: () => void } = $props();

  let url = $state('');
  let devtools = $state(false);
  let egress = $state('');
  let egresses: Egress[] = $state([]);
  let profile = $state('');
  let profiles: ProfileList | null = $state(null);
  let sessions: SessionInfo[] | null = $state(null);
  let error = $state('');
  let starting = $state(false);
  // A response that arrives after this view is gone must not navigate.
  let alive = true;
  $effect(() => () => (alive = false));

  async function load() {
    try {
      // The configured egresses, each with where its browsers say they are; the choice shows only when there are some.
      egresses = (await api<{ egresses: Egress[] }>('GET', '/api/egresses')).egresses;
      profiles = await api<ProfileList>('GET', '/api/profiles');
      sessions = (await api<{ sessions: SessionInfo[] }>('GET', '/api/sessions')).sessions;
    } catch (e) {
      if (!alive) return;
      if (statusOf(e) === 401) return onLogin();
      error = messageOf(e);
    }
  }
  $effect(() => {
    load();
  });

  async function start(event: SubmitEvent) {
    event.preventDefault();
    starting = true;
    error = '';
    try {
      const body: Record<string, unknown> = { url, devtools };
      if (egress !== '') body.egress = egress;
      if (profile !== '') body.profile = profile;
      const created = await api<SessionInfo>('POST', '/api/sessions', body);
      if (alive) onWatch(created.id);
    } catch (e) {
      error = statusOf(e) === 423 ? 'Saved profiles are locked: unlock them below with your password.' : messageOf(e);
      starting = false;
    }
  }

  async function close(id: string) {
    await api('DELETE', `/api/sessions/${id}`).catch((e: unknown) => (error = messageOf(e)));
    load();
  }

  const flags = (s: SessionInfo) =>
    [
      s.devtools ? 'DevTools' : 'no DevTools',
      s.egress ? `via ${s.egress}` : '',
      s.profile ? `profile ${s.profile.name}` : '',
      s.failed ? `failed: ${s.failed}` : '',
    ]
      .filter(Boolean)
      .join(' · ');
</script>

<div class="mx-auto mt-8 grid max-w-[76rem] gap-4 lg:grid-cols-[24rem_1fr]">
  <form class="{card} self-start" onsubmit={start}>
    <h1 class="text-xl font-semibold tracking-[-0.02em]">New browser session</h1>
    <label class={label} for="url">URL</label>
    <input id="url" class={field} type="url" placeholder="https://example.com" required bind:value={url} />
    {#if egresses.length > 0}
      <label class={label} for="egress">Leaves through</label>
      <select id="egress" class={field} bind:value={egress}>
        <option value="">Directly</option>
        {#each egresses as e (e.name)}<option value={e.name}>{e.name} ({e.timezone}, {e.languages[0]})</option>{/each}
      </select>
    {/if}
    {#if profiles && profiles.profiles.length > 0}
      <label class={label} for="profile">Profile</label>
      <select id="profile" class={field} bind:value={profile}>
        <option value="">None: a fresh browser</option>
        {#each profiles.profiles as p (p.id)}<option value={p.id} disabled={p.session !== null}>{choiceLabel(p)}</option>{/each}
      </select>
    {/if}
    <label class="mt-4 flex items-start gap-2.5 text-sm">
      <input class="mt-1 accent-accent" type="checkbox" bind:checked={devtools} />
      <span><span class="font-semibold">Allow DevTools for the agent</span><br /><span class="text-muted">Navigation and exact tab binding; Chromium listens on its own loopback only.</span></span>
    </label>
    <button class="{primary} mt-6 w-full" type="submit" disabled={starting}>Start session</button>
    <p class="mt-3 font-semibold text-bad" role="alert">{error}</p>
  </form>

  <section class={card}>
    <h2 class={panelTitle}>Sessions</h2>
    {#if sessions === null}
      <p class="text-muted">Loading…</p>
    {:else if sessions.length === 0}
      <p class="text-muted">No sessions.</p>
    {:else}
      <ul class="divide-y divide-line">
        {#each sessions as s (s.id)}
          <li class="flex items-center gap-3 py-3">
            <span class="min-w-0 flex-1">
              <span class="block truncate font-semibold">{s.url}</span>
              <span class="text-sm text-muted">{flags(s)}</span>
            </span>
            <button class={button} type="button" onclick={() => onWatch(s.id)}>Watch</button>
            <button class={button} type="button" onclick={() => close(s.id)}>Close</button>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <Profiles onChange={load} {onLogin} />

  <Tokens />
</div>
