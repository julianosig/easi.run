<script lang="ts">
  // Which account opens a session's chat (admin/src/chat.rs), and the accounts themselves: several of each program, each
  // named by the person, each signing in inside its own terminal the first time it opens.
  import { t, type Said } from '../i18n/index.svelte.ts';
  import { rowBadge } from './admin.ts';
  import { api, type ChatAccount, type ChatInfo } from './api.ts';
  import Button from './Button.svelte';
  import Dialog from './Dialog.svelte';
  import { errorOf } from './errors.ts';
  import Field from './Field.svelte';
  import Icon from './Icon.svelte';
  import { ICONS } from './icons.ts';
  import { field } from './ui.ts';

  let {
    id,
    open = $bindable(false),
    onStarted,
  }: { id: string; open?: boolean; onStarted: (chat: ChatInfo) => void } = $props();

  let accounts: ChatAccount[] = $state([]);
  let error: Said | null = $state(null);
  let starting: string | null = $state(null);
  let name = $state('');
  let product: ChatInfo['product'] = $state('codex');
  const fieldId = $props.id();

  const read = async () => {
    try {
      accounts = (await api<{ accounts: ChatAccount[] }>('GET', '/api/chat-accounts')).accounts;
    } catch (e) {
      error = errorOf(e);
    }
  };
  $effect(() => {
    if (open) {
      error = null;
      void read();
    }
  });

  async function add(e: SubmitEvent) {
    e.preventDefault();
    error = null;
    try {
      await api('POST', '/api/chat-accounts', { name, product });
      name = '';
      await read();
    } catch (failure) {
      error = errorOf(failure);
    }
  }

  async function remove(account: ChatAccount) {
    if (!confirm(t('chat.remove.confirm', { name: account.name }))) return;
    error = null;
    try {
      await api('DELETE', `/api/chat-accounts/${account.id}`);
      await read();
    } catch (failure) {
      error = errorOf(failure);
    }
  }

  async function start(account: ChatAccount) {
    error = null;
    starting = account.id;
    try {
      const { chat } = await api<{ chat: ChatInfo }>('POST', `/api/sessions/${id}/chat`, { account: account.id });
      open = false;
      onStarted(chat);
    } catch (failure) {
      error = errorOf(failure);
    } finally {
      starting = null;
    }
  }
</script>

<Dialog title={t('chat.picker.title')} icon={ICONS.chat} subtitle={t('chat.picker.subtitle')} bind:open>
  <p class="mt-4 font-semibold text-bad empty:hidden" role="alert">{error?.()}</p>
  {#if accounts.length === 0}
    <p class="mt-4 text-sm text-muted">{t('chat.empty')}</p>
  {:else}
    <ul class="mt-4 grid gap-2" aria-label={t('chat.accounts')}>
      {#each accounts as account (account.id)}
        <li class="flex items-center gap-3 rounded-xl p-2 ring-1 ring-line">
          <span class="{rowBadge} bg-wash text-muted"><Icon d={ICONS.chat} class="size-5" /></span>
          <span class="min-w-0 flex-1">
            <span class="block truncate font-semibold">{account.name}</span>
            <span class="block text-[13px] text-muted">
              {t(`chat.product.${account.product}`)}{account.session ? ` · ${t('chat.inUse')}` : ''}
            </span>
          </span>
          <Button size="sm" icon={ICONS.start} disabled={starting !== null || !!account.session} onclick={() => start(account)}>
            {t(starting === account.id ? 'chat.opening' : 'chat.use')}
          </Button>
          <Button variant="ghost" tone="danger" size="sm" icon={ICONS.delete} label={t('chat.remove', { name: account.name })} onclick={() => remove(account)} />
        </li>
      {/each}
    </ul>
  {/if}
  <form class="mt-2 grid gap-x-2 sm:grid-cols-[minmax(0,1fr)_11rem]" onsubmit={add}>
    <Field icon={ICONS.edit} label={t('chat.add.name')} for="{fieldId}-name">
      <input id="{fieldId}-name" class={field} placeholder={t('chat.add.name.placeholder')} maxlength="64" required bind:value={name} />
    </Field>
    <Field icon={ICONS.chat} label={t('chat.add.program')} for="{fieldId}-product">
      <select id="{fieldId}-product" class={field} bind:value={product}>
        <option value="codex">{t('chat.product.codex')}</option>
        <option value="claude">{t('chat.product.claude')}</option>
      </select>
    </Field>
    <div class="mt-3 flex justify-end sm:col-span-2"><Button type="submit" icon={ICONS.add}>{t('chat.add')}</Button></div>
  </form>
</Dialog>
