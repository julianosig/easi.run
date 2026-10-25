<script lang="ts">
  // The administrator's agent tokens: create one (shown once, to copy), list them, and revoke one, which stops every
  // session it started.
  import { api, copyText, messageOf, type TokenInfo } from './api.ts';
  import { button, card, field, label, panelTitle, primary } from './ui.ts';

  let tokens: TokenInfo[] | null = $state(null);
  let name = $state('');
  let created: { name: string; token: string } | null = $state(null);
  let copied = $state(false);
  let busy = $state(false);
  let error = $state('');

  async function load() {
    try {
      tokens = (await api<{ tokens: TokenInfo[] }>('GET', '/api/tokens')).tokens;
    } catch (e) {
      error = messageOf(e);
    }
  }
  $effect(() => {
    load();
  });

  async function create(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = '';
    copied = false;
    try {
      const made = await api<TokenInfo & { token: string }>('POST', '/api/tokens', { name });
      created = { name: made.name, token: made.token };
      name = '';
      load();
    } catch (e) {
      error = messageOf(e);
    }
    busy = false;
  }

  async function copy() {
    if (!created) return;
    if (await copyText(created.token)) copied = true;
    else error = 'Copying was refused; select the token and copy it yourself.';
  }

  async function revoke(token: TokenInfo) {
    if (!confirm(`Revoke “${token.name}”? Every session it started stops now.`)) return;
    error = '';
    await api('DELETE', `/api/tokens/${token.id}`).catch((e: unknown) => (error = messageOf(e)));
    load();
  }

  const when = (seconds: number) => new Date(seconds * 1000).toLocaleString();
</script>

<section class="{card} lg:col-span-2">
  <h2 class={panelTitle}>Agent tokens</h2>
  <p class="text-sm text-muted">
    A token lets an agent start its own sessions and use their desktop tools, through the agent API or MCP at
    <code>/mcp</code>. It never reaches your other sessions, the viewer, or these settings; you can watch, take over and
    stop its sessions here.
  </p>
  <form class="mt-2 flex flex-wrap items-end gap-3" onsubmit={create}>
    <span class="min-w-48 flex-1">
      <label class={label} for="token-name">Name</label>
      <input id="token-name" class={field} type="text" maxlength="64" placeholder="My laptop’s agent" required bind:value={name} />
    </span>
    <button class={primary} type="submit" disabled={busy}>Create token</button>
  </form>
  {#if created}
    <div class="mt-4 rounded-xl border border-accent/40 bg-accent-soft/40 p-4">
      <p class="text-sm font-semibold">“{created.name}” — copy it now: it is not shown again.</p>
      <div class="mt-2 flex flex-wrap items-center gap-3">
        <code class="min-w-0 flex-1 select-all break-all font-mono text-sm">{created.token}</code>
        <button class={button} type="button" onclick={copy}>{copied ? 'Copied' : 'Copy'}</button>
        <button class={button} type="button" onclick={() => (created = null)}>Done</button>
      </div>
    </div>
  {/if}
  <p class="mt-3 font-semibold text-bad" role="alert">{error}</p>
  {#if tokens === null}
    <p class="text-muted">Loading…</p>
  {:else if tokens.length === 0}
    <p class="text-muted">No tokens.</p>
  {:else}
    <ul class="divide-y divide-line">
      {#each tokens as t (t.id)}
        <li class="flex items-center gap-3 py-3">
          <span class="min-w-0 flex-1">
            <span class="block truncate font-semibold">{t.name}</span>
            <span class="text-sm text-muted">
              {t.revoked ? 'Revoking did not finish; try again' : `Created ${when(t.created)} · ${t.last_used ? `last used ${when(t.last_used)}` : 'never used'}`}
            </span>
          </span>
          <button class={button} type="button" onclick={() => revoke(t)}>Revoke</button>
        </li>
      {/each}
    </ul>
  {/if}
</section>
