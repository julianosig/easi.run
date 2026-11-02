<script lang="ts">
  // The administrator's webhook (admin/src/notify.rs): set it (its signing secret is shown once, to copy), send it a
  // test, or remove it. The admin posts to it when a person is needed: an agent asks for one, an agent's file waits for
  // an answer, or an agent finds the saved sessions locked.
  import Button from './Button.svelte';
  import Field from './Field.svelte';
  import Icon from './Icon.svelte';
  import Skeleton from './Skeleton.svelte';
  import { ICONS } from './icons.ts';
  import { type Said, t } from '../i18n/index.svelte.ts';
  import { api, copyText } from './api.ts';
  import { requests } from './requests.svelte.ts';
  import { field } from './ui.ts';

  let { onLogin }: { onLogin: () => void } = $props();
  const page = requests(() => onLogin());
  /** The webhook set, `null` for none, `undefined` until the admin answered. */
  let url: string | null | undefined = $state(undefined);
  let editing = $state(false);
  let draft = $state('');
  let busy = $state(false);
  let error: Said | null = $state(null);
  /** The secret of the webhook just set, shown until the page is left. */
  let secret: string | null = $state(null);
  let copied = $state(false);
  let tested: Said | null = $state(null);

  $effect(() => {
    (async () => {
      try {
        const got = await page.latest(() => api<{ url: string | null }>('GET', '/api/notifications'));
        if (got) url = got.url;
      } catch (e) {
        page.fail(e, (said) => (error = said));
      }
    })();
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = tested = null;
    try {
      const set = await api<{ url: string; secret: string }>('PUT', '/api/notifications', { url: draft });
      if (!page.alive()) return;
      url = set.url;
      secret = set.secret;
      copied = false;
      editing = false;
    } catch (e) {
      page.fail(e, (said) => (error = said));
    }
    busy = false;
  }

  async function test() {
    busy = true;
    error = tested = null;
    try {
      const answered = await api<{ status: number }>('POST', '/api/notifications/test');
      if (page.alive()) tested = () => t('agents.notify.tested', { status: answered.status });
    } catch (e) {
      page.fail(e, (said) => (error = said));
    }
    busy = false;
  }

  async function remove() {
    if (!confirm(t('agents.notify.remove.confirm'))) return;
    busy = true;
    error = tested = null;
    try {
      await api('DELETE', '/api/notifications');
      if (page.alive()) [url, secret] = [null, null];
    } catch (e) {
      page.fail(e, (said) => (error = said));
    }
    busy = false;
  }

  async function copy() {
    if (secret) copied = await copyText(secret);
  }
</script>

<section class="mt-12" aria-labelledby="notify-title">
  <h2 id="notify-title" class="flex items-center gap-2 text-[17px] font-semibold tracking-tight">{t('agents.notify.title')}</h2>
  <p class="mt-1 text-sm text-muted">{t('agents.notify.subtitle')}</p>
  <p class="mt-3 font-semibold text-bad empty:hidden" role="alert">{error?.()}</p>
  {#if url === undefined}
    {#if !error}<Skeleton said={t('agents.notify.loading')} />{/if}
  {:else if url === null || editing}
    <form class="mt-3 flex flex-wrap items-end gap-3" onsubmit={save}>
      <div class="min-w-0 flex-1">
        <Field icon={ICONS.link} label={t('agents.notify.url')} for="notify-url" help={t('agents.notify.url.help')}>
          <input id="notify-url" class={field} type="url" inputmode="url" placeholder="https://hooks.slack.com/services/…" required aria-describedby="notify-url-help" bind:value={draft} />
        </Field>
      </div>
      <div class="flex gap-2">
        {#if editing}<Button disabled={busy} onclick={() => (editing = false)}>{t('agents.notify.cancel')}</Button>{/if}
        <Button variant="primary" icon={ICONS.check} type="submit" disabled={busy}>{t('agents.notify.save')}</Button>
      </div>
    </form>
  {:else}
    <div class="mt-3 flex flex-wrap items-center gap-3 rounded-xl px-3 py-3 transition hover:bg-wash focus-within:bg-wash sm:flex-nowrap">
      <span class="grid size-9 shrink-0 place-items-center rounded-xl bg-accent-soft text-accent"><Icon d={ICONS.link} class="size-[18px] shrink-0" /></span>
      <span class="min-w-0 flex-1">
        <span class="block text-[15px] font-semibold">{t('agents.notify.set')}</span>
        <span class="block truncate text-[13px] text-muted">{url}</span>
      </span>
      <div class="flex flex-wrap gap-2">
        <Button size="sm" icon={ICONS.send} disabled={busy} onclick={test}>{t('agents.notify.test')}</Button>
        <Button size="sm" disabled={busy} onclick={() => ((draft = url ?? ''), (editing = true))}>{t('agents.notify.change')}</Button>
        <Button variant="ghost" tone="danger" size="sm" icon={ICONS.delete} label={t('agents.notify.remove')} disabled={busy} onclick={remove} />
      </div>
    </div>
    <p class="mt-2 text-sm font-semibold text-good empty:hidden" role="status">{tested?.()}</p>
    {#if secret}
      <div class="mt-3 rounded-xl bg-wash px-4 py-3">
        <p class="text-sm font-semibold">{t('agents.notify.secret')}</p>
        <p class="mt-1 text-[13px] text-muted">{t('agents.notify.secret.help')}</p>
        <code class="mt-2 block select-all break-all font-mono text-sm">{secret}</code>
        <div class="mt-3"><Button size="sm" icon={copied ? ICONS.check : ICONS.copy} onclick={copy}>{copied ? t('agents.copied') : t('agents.copy')}</Button></div>
      </div>
    {/if}
  {/if}
</section>
