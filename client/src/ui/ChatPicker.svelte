<script lang="ts">
  // Which account opens a session's chat (admin/src/chat.rs): picking one is all the first dialog asks. Another adds an
  // account, its provider and then its email, and opens it; each signs in inside its own terminal the first time. With no
  // account yet, that is the one that opens. A third removes them.
  import { t, type Said } from '../i18n/index.svelte.ts';
  import { rowBadge } from './admin.ts';
  import { api, type ChatAccount, type ChatInfo } from './api.ts';
  import Button from './Button.svelte';
  import Dialog from './Dialog.svelte';
  import { errorOf } from './errors.ts';
  import Field from './Field.svelte';
  import { ICONS } from './icons.ts';
  import ProductLogo from './ProductLogo.svelte';
  import { field } from './ui.ts';

  let {
    id,
    open = $bindable(false),
    onStarted,
  }: { id: string; open?: boolean; onStarted: (chat: ChatInfo) => void } = $props();

  let accounts: ChatAccount[] = $state([]);
  let loaded = $state(false);
  let error: Said | null = $state(null);
  let starting: string | null = $state(null);
  let adding = $state(false);
  let managing = $state(false);
  let email = $state('');
  /** The provider of the account being added: its email is asked once this is picked. */
  let product: ChatInfo['product'] | null = $state(null);
  let emailField: HTMLInputElement | undefined = $state();
  const PROVIDERS = [
    { id: 'codex', maker: 'OpenAI' },
    { id: 'claude', maker: 'Anthropic' },
  ] as const;
  /** Whether there is a choice to show: not under another dialog, where it would come back on top as accounts change. */
  const picking = $derived(open && loaded && accounts.length > 0 && !adding && !managing);
  const fieldId = $props.id();

  $effect(() => {
    if (!adding) {
      product = null;
      email = '';
    }
  });
  $effect(() => {
    if (product) emailField?.focus();
  });

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
      loaded = false;
      void read().then(() => {
        loaded = true;
        if (accounts.length === 0) adding = true;
      });
    }
  });

  /** Adds the account and opens it here: signing in happens in its terminal. */
  async function connect(e: SubmitEvent) {
    e.preventDefault();
    error = null;
    try {
      const account = await api<ChatAccount>('POST', '/api/chat-accounts', { name: email, product });
      email = '';
      await read();
      await start(account);
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
      if (accounts.length === 0) {
        managing = false;
        adding = true;
      }
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
      adding = false;
      onStarted(chat);
    } catch (failure) {
      error = errorOf(failure);
    } finally {
      starting = null;
    }
  }
</script>

<Dialog
  title={t('chat.picker.title')}
  icon={ICONS.chat}
  bind:open={() => picking, (shown) => (open = shown)}
>
  <p class="mt-4 font-semibold text-bad empty:hidden" role="alert">{error?.()}</p>
  <ul class="mt-4 grid gap-2" aria-label={t('chat.accounts')}>
    {#each accounts as account (account.id)}
      <li>
        <button
          type="button"
          class="flex w-full cursor-pointer items-center gap-3 rounded-xl p-2 text-left ring-1 ring-line transition-colors hover:bg-wash disabled:cursor-default disabled:opacity-60"
          disabled={starting !== null}
          onclick={() => start(account)}
        >
          <span class="{rowBadge} bg-wash"><ProductLogo product={account.product} /></span>
          <span class="min-w-0 flex-1">
            <span class="block truncate font-semibold">{account.name}</span>
            <span class="block text-[13px] text-muted">{t(starting === account.id ? 'chat.opening' : `chat.product.${account.product}`)}</span>
          </span>
        </button>
      </li>
    {/each}
  </ul>
  <div class="mt-4 flex justify-end gap-2">
    <Button variant="ghost" size="sm" icon={ICONS.edit} onclick={() => ((error = null), (managing = true))}>{t('chat.manage')}</Button>
    <Button variant="ghost" size="sm" icon={ICONS.add} onclick={() => ((error = null), (adding = true))}>{t('chat.add')}</Button>
  </div>
</Dialog>

<Dialog title={t('chat.accounts')} icon={ICONS.chat} bind:open={managing}>
  <p class="mt-4 font-semibold text-bad empty:hidden" role="alert">{error?.()}</p>
  {#if accounts.length > 0}
    <ul class="mt-4 grid gap-2" aria-label={t('chat.accounts')}>
      {#each accounts as account (account.id)}
        <li class="flex items-center gap-3 rounded-xl p-2 ring-1 ring-line">
          <span class="{rowBadge} bg-wash"><ProductLogo product={account.product} /></span>
          <span class="min-w-0 flex-1">
            <span class="block truncate font-semibold">{account.name}</span>
            <span class="block text-[13px] text-muted">
              {t(`chat.product.${account.product}`)}{account.sessions.length ? ` · ${t('chat.inUse')}` : ''}
            </span>
          </span>
          <Button variant="ghost" tone="danger" size="sm" icon={ICONS.delete} label={t('chat.remove', { name: account.name })} onclick={() => remove(account)} />
        </li>
      {/each}
    </ul>
  {/if}
</Dialog>

<Dialog
  title={t(product ? 'chat.account.title' : 'chat.provider.title')}
  icon={ICONS.chat}
  subtitle={t(product ? 'chat.account.subtitle' : 'chat.provider.subtitle')}
  bind:open={
    () => adding,
    (shown) => {
      adding = shown;
      // With no account to pick from, nothing is left to go back to.
      if (!shown && accounts.length === 0) open = false;
    }
  }
>
  <p class="mt-4 font-semibold text-bad empty:hidden" role="alert">{error?.()}</p>
  {#if product}
    <form class="mt-4" onsubmit={connect}>
      <Field icon={ICONS.email} label={t('chat.add.email')} for="{fieldId}-email">
        <input id="{fieldId}-email" class={field} type="email" autocomplete="off" placeholder={t('chat.add.email')} maxlength="64" required bind:this={emailField} bind:value={email} />
      </Field>
      <div class="mt-3 flex justify-end gap-2">
        <Button variant="ghost" onclick={() => (product = null)}>{t('chat.back')}</Button>
        <Button type="submit" icon={ICONS.start} disabled={starting !== null}>{t(starting ? 'chat.opening' : 'chat.connect')}</Button>
      </div>
    </form>
  {:else}
    <ul class="mt-4 grid gap-2">
      {#each PROVIDERS as provider (provider.id)}
        <li>
          <button
            type="button"
            class="flex w-full cursor-pointer items-center gap-3 rounded-xl p-2 text-left ring-1 ring-line transition-colors hover:bg-wash"
            onclick={() => (product = provider.id)}
          >
            <span class="{rowBadge} bg-wash"><ProductLogo product={provider.id} class="size-6" /></span>
            <span class="min-w-0 flex-1">
              <span class="block truncate font-semibold">{t(`chat.product.${provider.id}`)}</span>
              <span class="block text-[13px] text-muted">{provider.maker}</span>
            </span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</Dialog>
