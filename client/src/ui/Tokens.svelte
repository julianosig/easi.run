<script lang="ts">
  // The administrator's agent tokens: create one (shown once, to copy), list them, and revoke one, which stops every
  // session it started.
  import { i18n, t } from '../i18n/index.svelte.ts';
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
    else createError = t('agents.created.refused');
  }

  async function revoke(token: TokenInfo) {
    menu = null;
    if (!confirm(t('agents.revoke.confirm', { name: token.name }))) return;
    error = '';
    await api('DELETE', `/api/tokens/${token.id}`).catch((e: unknown) => page.fail(e, (text) => (error = text)));
    load();
  }

  const when = (seconds: number) => new Date(seconds * 1000).toLocaleString(i18n.locale);
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

<PageHeader icon={PAGE_ICONS.agents} title={t('agents.title')} subtitle={t('agents.subtitle')}>
  {#snippet action()}
    {#if tokens}
      <NewButton label={t('agents.new')} onclick={newToken} />
    {/if}
  {/snippet}
</PageHeader>
<p class="mt-3 text-sm text-muted">{t('agents.mcp.before')} <code class="rounded-md bg-wash px-1.5 py-0.5 font-mono text-[13px] text-ink">/mcp</code>{t('agents.mcp.after')}</p>

<p class="mt-6 font-semibold text-bad empty:hidden" role="alert">{error}</p>

{#if tokens === null}
  {#if !error}<Skeleton said={t('agents.loading')} />{/if}
{:else if tokens.length === 0}
  <EmptyState
    icon={FORM_ICONS.key}
    title={t('agents.empty.title')}
    text={t('agents.empty.text')}
    points={[t('agents.empty.watch'), t('agents.empty.revoke')]}
  />
{:else}
  <ul class="mt-8 {rows}">
    {#each tokens as token (token.id)}
      <li class="flex items-center gap-4 py-3 {row}">
        <span class={rowBadge}><Icon d={FORM_ICONS.key} class="size-5" /></span>
        <span class="min-w-0 flex-1">
          <span class="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span class="truncate font-semibold">{token.name}</span>
            {#if token.revoked}<span class="{chip} {CHIP_TONES.bad}">{t('agents.token.unfinished')}</span>{:else if token.last_used}<span class="{chip} {CHIP_TONES.good}">{t('agents.token.used', { when: when(token.last_used) })}</span>{:else}<span class="{chip} {CHIP_TONES.neutral}">{t('agents.token.unused')}</span>{/if}
          </span>
          <span class="mt-1 block text-sm text-muted">{token.revoked ? t('agents.token.finish') : t('agents.token.created', { when: when(token.created) })}</span>
        </span>
        <Menu label={t('agents.token.more', { name: token.name })} bind:open={() => menu === token.id, (on) => (menu = on ? token.id : null)}>
          <button class="{menuItem} text-bad" type="button" onclick={() => revoke(token)}>{t('agents.revoke')}</button>
        </Menu>
      </li>
    {/each}
  </ul>
{/if}

<Dialog
  title={created ? t('agents.created.title') : t('agents.create.title')}
  icon={created ? FORM_ICONS.key : PAGE_ICONS.agents}
  subtitle={created ? t('agents.created.subtitle') : t('agents.create.subtitle')}
  bind:open={() => creating, (on) => busy || (creating = on)}
>
  {#if created}
    <p class="mt-5 text-sm font-semibold">{t('agents.created.copyNow', { name: created.name })}</p>
    <code class="mt-2 block select-all break-all rounded-xl bg-wash px-4 py-3 font-mono text-sm">{created.token}</code>
    <p class="mt-3 text-sm font-semibold text-bad empty:hidden" role="alert">{createError}</p>
    <div class="mt-6 flex justify-end gap-3">
      <button class={button} type="button" bind:this={copyButton} onclick={copy}>{copied ? t('agents.copied') : t('agents.copy')}</button>
      <button class={primary} type="button" onclick={() => (creating = false)}>{t('agents.created.done')}</button>
    </div>
  {:else}
    <form onsubmit={create}>
      <Field icon={FORM_ICONS.name} label={t('agents.create.name')} for="token-name" help={t('agents.create.name.help')}>
        <input id="token-name" class={field} type="text" maxlength="64" placeholder={t('agents.create.name.placeholder')} required aria-describedby="token-name-help" bind:value={name} />
      </Field>
      <p class="mt-4 text-sm font-semibold text-bad empty:hidden" role="alert">{createError}</p>
      <div class="mt-7 flex justify-end gap-3">
        <button class={button} type="button" disabled={busy} onclick={() => (creating = false)}>{t('agents.create.cancel')}</button>
        <button class="{primary} inline-flex items-center gap-2" type="submit" disabled={busy}><Icon d={FORM_ICONS.key} class="shrink-0 size-[18px]" />{t('agents.create.submit')}</button>
      </div>
    </form>
  {/if}
</Dialog>
