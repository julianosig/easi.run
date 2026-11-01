<script lang="ts" module>
  /** The file permissions' saves, one after another across every Agents page this tab opens: a page left with a save
   * still queued never overwrites the next page's. */
  let saves = Promise.resolve();
</script>

<script lang="ts">
  // The administrator's agent tokens: create one (shown once, to copy), list them, and revoke one, which stops every
  // session it started. Under them, what every agent may do with files, which a change sets at once.
  import Button from './Button.svelte';
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { CHIP_TONES, chip, row, rowBadge, rows } from './admin.ts';
  import PageHeader from './PageHeader.svelte';
  import { type AgentFiles, api, copyText, type FileChoice, formatTime, type TokenInfo } from './api.ts';
  import { requests } from './requests.svelte.ts';
  import Dialog from './Dialog.svelte';
  import EmptyState from './EmptyState.svelte';
  import Field from './Field.svelte';
  import Icon from './Icon.svelte';
  import { tick } from 'svelte';
  import { field } from './ui.ts';
  import Skeleton from './Skeleton.svelte';
  import Switch from './Switch.svelte';

  let { onLogin }: { onLogin: () => void } = $props();
  let tokens: TokenInfo[] | null = $state(null);
  let name = $state('');
  let created: { name: string; token: string } | null = $state(null);
  let copied = $state(false);
  let busy = $state(false);
  let error: Said | null = $state(null);
  /** The new token's dialog (its name, then the token to copy), and what went wrong in it. */
  let creating = $state(false);
  let createError: Said | null = $state(null);
  let copyButton: HTMLButtonElement | undefined = $state();
  const page = requests(() => onLogin());

  async function load() {
    try {
      const listed = await page.latest(() => api<{ tokens: TokenInfo[] }>('GET', '/api/tokens'));
      if (listed) tokens = listed.tokens;
    } catch (e) {
      page.fail(e, (said) => (error = said));
    }
  }
  $effect(() => {
    load();
  });

  /** The file permissions as the page shows them, and as the admin last confirmed them. */
  let files: AgentFiles | null = $state.raw(null);
  let confirmed: AgentFiles | null = null;
  let filesError: Said | null = $state(null);
  // Read in the same queue as the saves, so a save an earlier page left under way lands first.
  $effect(() => {
    saves = saves.then(async () => {
      try {
        const got = await api<AgentFiles>('GET', '/api/agent-files');
        if (page.alive()) files = confirmed = got;
      } catch (e) {
        page.fail(e, (said) => (filesError = said));
      }
    });
  });
  /** Saves are made one after another, so the last change is what the admin keeps; each saves the page's permissions
   * as they are then, and one a later change overtook is skipped. A refused save puts back what the admin has. */
  function setFiles(change: Partial<AgentFiles>) {
    if (!files) return;
    const wanted = (files = { ...files, ...change });
    filesError = null;
    saves = saves.then(async () => {
      // Gone, or overtaken by a later change (which saves this one's too): nothing to send.
      if (!page.alive() || files !== wanted) return;
      try {
        const saved = await api<AgentFiles>('PUT', '/api/agent-files', wanted);
        confirmed = saved;
        if (page.alive() && files === wanted) files = saved;
      } catch (e) {
        // A later change, still to be saved, says how it went instead.
        if (files !== wanted) return;
        files = confirmed;
        page.fail(e, (said) => (filesError = said));
      }
    });
  }
  const CHOICES: FileChoice[] = ['ask', 'allow', 'deny'];

  async function create(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    createError = null;
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
      page.fail(e, (said) => (createError = said));
    }
    busy = false;
  }

  /** What a refused copy says; a copy that worked afterwards takes it away. */
  const copyRefused: Said = () => t('agents.created.refused');
  async function copy() {
    const shown = created;
    if (!shown) return;
    const worked = await copyText(shown.token);
    // Another token's dialog (or none) by now: this copy's outcome is not that token's.
    if (created !== shown) return;
    copied = worked;
    if (!copied) createError = copyRefused;
    else if (createError === copyRefused) createError = null;
  }

  async function revoke(token: TokenInfo) {
    if (!confirm(t('agents.revoke.confirm', { name: token.name }))) return;
    error = null;
    await api('DELETE', `/api/tokens/${token.id}`).catch((e: unknown) => page.fail(e, (said) => (error = said)));
    load();
  }

  const when = (seconds: number) => formatTime(seconds, i18n.locale);
  function newToken() {
    created = null;
    createError = null;
    creating = true;
  }
  // The token is shown once, in its dialog: closed, it goes from the page too.
  $effect(() => {
    if (!creating) created = null;
  });
</script>

{#snippet connect()}
  {t('agents.mcp.before')} <code class="rounded-md bg-paper px-1.5 py-0.5 font-mono text-[13px] text-ink">/mcp</code>{t('agents.mcp.after')}
{/snippet}

<PageHeader icon={ICONS.agents} title={t('agents.title')} subtitle={t('agents.subtitle')} add={tokens ? { label: t('agents.new'), onclick: newToken } : undefined} />

<p class="mt-6 font-semibold text-bad empty:hidden" role="alert">{error?.()}</p>

{#if tokens === null}
  {#if !error}<Skeleton said={t('agents.loading')} />{/if}
{:else if tokens.length === 0}
  <EmptyState points={[t('agents.empty.watch'), t('agents.empty.revoke'), connect]} />
{:else}
  <ul class="mt-8 {rows}">
    {#each tokens as token (token.id)}
      <li class="flex items-center gap-4 py-3 {row}">
        <span class={rowBadge}><Icon d={ICONS.token} class="size-5" /></span>
        <span class="min-w-0 flex-1">
          <span class="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span class="truncate font-semibold">{token.name}</span>
            {#if token.revoked}<span class="{chip} {CHIP_TONES.bad}">{t('agents.token.unfinished')}</span>{:else if token.last_used}<span class="{chip} {CHIP_TONES.good}">{t('agents.token.used', { when: when(token.last_used) })}</span>{:else}<span class="{chip} {CHIP_TONES.neutral}">{t('agents.token.unused')}</span>{/if}
          </span>
          <span class="mt-1 block text-sm text-muted">{token.revoked ? t('agents.token.finish') : t('agents.token.created', { when: when(token.created) })}</span>
        </span>
        <Button variant="ghost" tone="danger" size="sm" icon={ICONS.delete} label={t('agents.revoke', { name: token.name })} onclick={() => revoke(token)} />
      </li>
    {/each}
  </ul>
{/if}

{#snippet choice(key: 'downloads' | 'transfers', icon: string)}
  {@const id = `files-${key}`}
  <div class="flex flex-wrap items-center gap-3 rounded-xl px-3 py-3 transition hover:bg-wash focus-within:bg-wash sm:flex-nowrap">
    <span class="grid size-9 shrink-0 place-items-center rounded-xl bg-accent-soft text-accent"><Icon d={icon} class="size-[18px] shrink-0" /></span>
    <span class="min-w-0 flex-1">
      <label class="block text-[15px] font-semibold" for={id}>{t(`agents.files.${key}`)}</label>
      <span id="{id}-help" class="block text-[13px] leading-snug text-muted">{t(`agents.files.${key}.help`)}</span>
    </span>
    <select {id} class="{field} w-auto shrink-0" aria-describedby="{id}-help" value={files?.[key]} onchange={(e) => setFiles({ [key]: e.currentTarget.value as FileChoice })}>
      {#each CHOICES as c (c)}<option value={c}>{t(`agents.files.choice.${c}`)}</option>{/each}
    </select>
  </div>
{/snippet}

<section class="mt-12" aria-labelledby="files-title">
  <h2 id="files-title" class="flex items-center gap-2 text-[17px] font-semibold tracking-tight">{t('agents.files.title')}</h2>
  <p class="mt-1 text-sm text-muted">{t('agents.files.subtitle')}</p>
  <p class="mt-3 font-semibold text-bad empty:hidden" role="alert">{filesError?.()}</p>
  {#if files}
    <div class="mt-3 grid gap-1">
      {@render choice('downloads', ICONS.download)}
      {@render choice('transfers', ICONS.send)}
      <Switch icon={ICONS.upload} title={t('agents.files.uploads')} help={t('agents.files.uploads.help')} checked={files.uploads === 'allow'} onchange={(on) => setFiles({ uploads: on ? 'allow' : 'deny' })} />
    </div>
  {:else if !filesError}
    <Skeleton said={t('agents.files.loading')} />
  {/if}
</section>

<Dialog
  title={created ? t('agents.created.title') : t('agents.create.title')}
  icon={created ? ICONS.token : ICONS.agents}
  subtitle={created ? t('agents.created.subtitle') : t('agents.create.subtitle')}
  bind:open={() => creating, (on) => busy || (creating = on)}
>
  {#if created}
    <p class="mt-5 text-sm font-semibold">{t('agents.created.copyNow', { name: created.name })}</p>
    <code class="mt-2 block select-all break-all rounded-xl bg-wash px-4 py-3 font-mono text-sm">{created.token}</code>
    <p class="mt-3 text-sm font-semibold text-bad empty:hidden" role="alert">{createError?.()}</p>
    <div class="mt-6 flex justify-end gap-3">
      <Button icon={copied ? ICONS.check : ICONS.copy} bind:ref={copyButton} onclick={copy}>{copied ? t('agents.copied') : t('agents.copy')}</Button>
      <Button variant="primary" icon={ICONS.check} onclick={() => (creating = false)}>{t('agents.created.done')}</Button>
    </div>
  {:else}
    <form onsubmit={create}>
      <Field icon={ICONS.name} label={t('agents.create.name')} for="token-name" help={t('agents.create.name.help')}>
        <input id="token-name" class={field} type="text" placeholder={t('agents.create.name.placeholder')} required aria-describedby="token-name-help" bind:value={name} />
      </Field>
      <p class="mt-4 text-sm font-semibold text-bad empty:hidden" role="alert">{createError?.()}</p>
      <div class="mt-7 flex justify-end gap-3">
        <Button disabled={busy} onclick={() => (creating = false)}>{t('agents.create.cancel')}</Button>
        <Button variant="primary" icon={ICONS.token} type="submit" disabled={busy}>{t('agents.create.submit')}</Button>
      </div>
    </form>
  {/if}
</Dialog>
