<script lang="ts">
  // The password asked once more, after a sign-in or a restart that left what it opens closed: the past sessions'
  // replays, and the sessions waiting to start again. `subtitle` says what it opens here.
  import Button from './Button.svelte';
  import Dialog from './Dialog.svelte';
  import Field from './Field.svelte';
  import { ICONS } from './icons.ts';
  import { type Said, t } from '../i18n/index.svelte.ts';
  import { api, statusOf } from './api.ts';
  import { errorOf } from './errors.ts';
  import { field } from './ui.ts';

  let {
    open = $bindable(false),
    subtitle,
    onUnlocked,
    onLogin,
  }: { open?: boolean; subtitle: string; onUnlocked: () => void; onLogin: () => void } = $props();

  const id = $props.id();
  let password = $state('');
  let error: Said | null = $state(null);
  let busy = $state(false);
  // Each opening starts clean, however the last one closed: no password left in the field, no refusal still said.
  $effect(() => {
    if (!open) [password, error] = ['', null];
  });

  async function unlock(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    try {
      await api('POST', '/api/profiles/unlock', { password });
      [password, open, error] = ['', false, null];
      onUnlocked();
    } catch (e) {
      if (statusOf(e) === 401) return onLogin();
      // A refusal that comes once the dialog was closed is said to nobody: the next opening starts clean.
      if (open) error = errorOf(e);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title={t('sessions.unlock')} icon={ICONS.lock} {subtitle} bind:open>
  <form onsubmit={unlock}>
    <Field icon={ICONS.password} label={t('sessions.unlock.password')} for="{id}-password">
      <input id="{id}-password" class={field} type="password" autocomplete="current-password" bind:value={password} />
    </Field>
    <p class="mt-4 text-sm font-semibold text-bad empty:hidden" role="alert">{error?.()}</p>
    <div class="mt-7 flex justify-end gap-3">
      <Button disabled={busy} onclick={() => (open = false)}>{t('sessions.cancel')}</Button>
      <Button variant="primary" icon={ICONS.unlock} type="submit" disabled={busy}>{t('sessions.unlock')}</Button>
    </div>
  </form>
</Dialog>
