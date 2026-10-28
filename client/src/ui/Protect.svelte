<script lang="ts">
  // Right after the password is created: a second step at sign-in, offered and suggested, never forced. An
  // authenticator app works everywhere; a passkey only where the browser offers one, which it does not on a page whose
  // certificate it does not trust (as the admin's own, reached through a tunnel, is not): there the app is offered in
  // its place. The password just created comes along, as adding a passkey asks for it again.
  import { ICONS } from './icons.ts';
  import Button from './Button.svelte';
  import { type Said, t } from '../i18n/index.svelte.ts';
  import { api, messageOf } from './api.ts';
  import AppSetup from './AppSetup.svelte';
  import RecoveryCodes from './RecoveryCodes.svelte';
  import { addPasskey, passkeyRefusal, passkeysInBrowser, type SecondStep, UNTRUSTED_PASSKEY } from './second.ts';
  import Gate from './Gate.svelte';
  import { gateLead, gateStepTitle, gateTitle, textLink } from './ui.ts';

  let { password, onDone }: { password: string; onDone: () => void } = $props();
  let step: 'choose' | 'app' | 'codes' = $state('choose');
  let codes: string[] = $state([]);
  /** The browser and the admin's address both allow passkeys, as far as can be known before trying one. */
  let passkeys = $state(false);
  let busy = $state(false);
  let note: Said | null = $state(null);
  let error: Said | null = $state(null);

  $effect(() => {
    api<SecondStep>('GET', '/api/security').then(
      (status) => (passkeys = status.passkeys_possible && passkeysInBrowser()),
      () => (passkeys = false),
    );
  });

  function added(issued: string[] | null) {
    if (issued) {
      codes = issued;
      step = 'codes';
    } else {
      onDone();
    }
  }

  async function passkey() {
    busy = true;
    error = null;
    note = null;
    try {
      added(await addPasskey(password));
    } catch (e) {
      const refused = passkeyRefusal(e);
      if (refused === 'untrusted') {
        passkeys = false;
        note = () => t(UNTRUSTED_PASSKEY);
      } else if (refused === 'cancelled') {
        error = () => t('account.protect.passkey.cancelled');
      } else {
        if (e instanceof DOMException) error = () => t('account.protect.passkey.failed');
        else {
          const message = messageOf(e);
          error = () => message;
        }
      }
    } finally {
      busy = false;
    }
  }
</script>

<Gate label={t('account.protect.label')}>
  {#if step === 'app'}
    <AppSetup heading={gateStepTitle} onDone={added} onCancel={() => (step = 'choose')} onLogin={onDone} />
  {:else if step === 'codes'}
    <RecoveryCodes heading={gateStepTitle} {codes} {onDone} />
  {:else}
    <h1 class={gateTitle}>{t('account.protect.title')}</h1>
    <p class={gateLead}>
      {t('account.protect.lead')}
    </p>
    <Button variant="primary" size="lg" icon={ICONS.app} class="mt-7" disabled={busy} onclick={() => (step = 'app')}>{t('account.protect.app')}</Button>
    <p class="mt-2 text-center text-sm text-muted">{t('account.protect.app.line')}</p>
    {#if passkeys}
      <Button size="lg" icon={ICONS.passkey} class="mt-5" disabled={busy} onclick={passkey}>{t('account.protect.passkey')}</Button>
      <p class="mt-2 text-center text-sm text-muted">{t('account.protect.passkey.line')}</p>
    {/if}
    <p class="mt-4 text-sm font-semibold" role="status">{note?.()}</p>
    <p class="mt-1 font-semibold text-bad" role="alert">{error?.()}</p>
    <div class="mt-8">
      <button class="w-full py-1 {textLink}" type="button" disabled={busy} onclick={onDone}>{t('account.protect.skip')}</button>
      <p class="mt-1 text-center text-sm text-muted">{t('account.protect.later')}</p>
    </div>
  {/if}
</Gate>
