<script lang="ts">
  // The administrator's agent tokens: create one (shown once, to copy), list them, and revoke one, which stops every
  // session it started.
  import { CHIP_TONES, chip, FORM_ICONS, menuItem, PAGE_ICONS, row, rowBadge, rows } from './admin.ts';
  import PageHeader from './PageHeader.svelte';
  import { api, copyText, type TokenInfo } from './api.ts';
  import { requests } from './requests.svelte.ts';
  import Dialog from './Dialog.svelte';
  import Field from './Field.svelte';
  import EmptyState from './EmptyState.svelte';
  import Icon from './Icon.svelte';
  import NewButton from './NewButton.svelte';
  import Menu from './Menu.svelte';
  import { tick } from 'svelte';
  import { button, field, primary } from './ui.ts';
  import Skeleton from './Skeleton.svelte';

  let { onLogin }: { onLogin: () => void } = $props();
  let tokens: TokenInfo[] | null = $state(null);
  let name = $state('');
  let created: { name: string; token: string } | null = $state(null);
  let copied = $state(false);
  let busy = $state(false);
  let error = $state('');
  /** The new token's dialog (its name, then the token to copy), and what went wrong in it. */
  let creating = $state(false);
  let createError = $state('');
  /** The row whose menu is open. */
  let menu: string | null = $state(null);
  let copyButton: HTMLButtonElement | undefined = $state();
  const page = requests(() => onLogin());

  async function load() {
    try {
      tokens = (await api<{ tokens: TokenInfo[] }>('GET', '/api/tokens')).tokens;
    } catch (e) {
      page.fail(e, (text) => (error = text));
    }
  }
  $effect(() => {
    load();
  });

  async function create(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    createError = '';
    copied = false;
    try {
      const made = await api<TokenInfo & { token: string }>('POST', '/api/tokens', { name });
      if (!page.alive()) return;
      created = { name: made.name, token: made.token };
      name = '';
      load();
      // The form went with the focus in it: on to the token's Copy.
      await tick();
      copyButton?.focus();
    } catch (e) {
      page.fail(e, (text) => (createError = text));
    }
    busy = false;
  }

  async function copy() {
    if (!created) return;
    if (await copyText(created.token)) copied = true;
    else createError = 'Copying was refused; select the token and copy it yourself.';
  }

  async function revoke(token: TokenInfo) {
    menu = null;
    if (!confirm(`Revoke “${token.name}”? Every session it started stops now.`)) return;
    error = '';
    await api('DELETE', `/api/tokens/${token.id}`).catch((e: unknown) => page.fail(e, (text) => (error = text)));
    load();
  }

  const when = (seconds: number) => new Date(seconds * 1000).toLocaleString();
  function newToken() {
    created = null;
    createError = '';
    creating = true;
  }
  // The token is shown once, in its dialog: closed, it goes from the page too.
  $effect(() => {
    if (!creating) created = null;
  });
</script>

<PageHeader icon={PAGE_ICONS.agents} title="Agents" subtitle="A token lets an agent start and use its own sessions — never yours, nor these settings.">
  {#snippet action()}
    {#if tokens && tokens.length > 0}
      <NewButton label="New token" onclick={newToken} />
    {/if}
  {/snippet}
</PageHeader>
<p class="mt-3 text-sm text-muted">Agents connect through the agent API or MCP at <code class="rounded-md bg-wash px-1.5 py-0.5 font-mono text-[13px] text-ink">/mcp</code>.</p>

<p class="mt-6 font-semibold text-bad empty:hidden" role="alert">{error}</p>

{#if tokens === null}
  {#if !error}<Skeleton said="Loading your agent tokens" />{/if}
{:else if tokens.length === 0}
  <EmptyState icon={FORM_ICONS.key} title="No agent tokens" text="Create one for each agent; you can watch, take over and stop its sessions in Sessions." label="New token" onclick={newToken} />
{:else}
  <ul class="mt-8 {rows}">
    {#each tokens as t (t.id)}
      <li class="flex items-center gap-4 py-3 {row}">
        <span class={rowBadge}><Icon d={FORM_ICONS.key} class="size-5" /></span>
        <span class="min-w-0 flex-1">
          <span class="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span class="truncate font-semibold">{t.name}</span>
            {#if t.revoked}<span class="{chip} {CHIP_TONES.bad}">Revoking did not finish</span>{:else if t.last_used}<span class="{chip} {CHIP_TONES.good}">Used {when(t.last_used)}</span>{:else}<span class="{chip} {CHIP_TONES.neutral}">Never used</span>{/if}
          </span>
          <span class="mt-1 block text-sm text-muted">{t.revoked ? 'Revoke it again to finish.' : `Created ${when(t.created)}`}</span>
        </span>
        <Menu label="More for {t.name}" bind:open={() => menu === t.id, (on) => (menu = on ? t.id : null)}>
          <button class="{menuItem} text-bad" type="button" onclick={() => revoke(t)}>Revoke</button>
        </Menu>
      </li>
    {/each}
  </ul>
{/if}

<Dialog
  title={created ? 'Token created' : 'New agent token'}
  icon={created ? FORM_ICONS.key : PAGE_ICONS.agents}
  subtitle={created ? 'Give it to your agent. It is shown only this once.' : 'Lets one agent start and use its own sessions.'}
  bind:open={() => creating, (on) => busy || (creating = on)}
>
  {#if created}
    <p class="mt-5 text-sm font-semibold">“{created.name}” — copy it now: it is not shown again.</p>
    <code class="mt-2 block select-all break-all rounded-xl bg-wash px-4 py-3 font-mono text-sm">{created.token}</code>
    <p class="mt-3 text-sm font-semibold text-bad empty:hidden" role="alert">{createError}</p>
    <div class="mt-6 flex justify-end gap-3">
      <button class={button} type="button" bind:this={copyButton} onclick={copy}>{copied ? 'Copied' : 'Copy'}</button>
      <button class={primary} type="button" onclick={() => (creating = false)}>Done</button>
    </div>
  {:else}
    <form onsubmit={create}>
      <Field icon={FORM_ICONS.name} label="Name" for="token-name" help="So you can tell your agents apart.">
        <input id="token-name" class={field} type="text" maxlength="64" placeholder="e.g. My laptop’s agent" required aria-describedby="token-name-help" bind:value={name} />
      </Field>
      <p class="mt-4 text-sm font-semibold text-bad empty:hidden" role="alert">{createError}</p>
      <div class="mt-7 flex justify-end gap-3">
        <button class={button} type="button" disabled={busy} onclick={() => (creating = false)}>Cancel</button>
        <button class="{primary} inline-flex items-center gap-2" type="submit" disabled={busy}><Icon d={FORM_ICONS.key} class="shrink-0 size-[18px]" />Create token</button>
      </div>
    </form>
  {/if}
</Dialog>
