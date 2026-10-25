<script lang="ts">
  // Right after the password is created: a second step at sign-in, offered and suggested, never forced. An
  // authenticator app works everywhere; a passkey only where the browser offers one, which it does not on a page whose
  // certificate it does not trust (as the admin's own, reached through a tunnel, is not): there the app is offered in
  // its place. The password just created comes along, as adding a passkey asks for it again.
  import { api, messageOf } from './api.ts';
  import AppSetup from './AppSetup.svelte';
  import RecoveryCodes from './RecoveryCodes.svelte';
  import { addPasskey, passkeyRefusal, passkeysInBrowser, type SecondStep } from './second.ts';
  import { button, card, focusRing, primary } from './ui.ts';

  let { password, onDone }: { password: string; onDone: () => void } = $props();
  let step: 'choose' | 'app' | 'codes' = $state('choose');
  let codes: string[] = $state([]);
  /** The browser and the admin's address both allow passkeys, as far as can be known before trying one. */
  let passkeys = $state(false);
  let busy = $state(false);
  let note = $state('');
  let error = $state('');

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
    error = '';
    note = '';
    try {
      added(await addPasskey(password));
    } catch (e) {
      const refused = passkeyRefusal(e);
      if (refused === 'untrusted') {
        passkeys = false;
        note =
          'This browser offers passkeys only on a site with a certificate it trusts, which this admin, with its own ' +
          'certificate, is not. Use an authenticator app instead: it works here.';
      } else if (refused === 'cancelled') {
        error = 'No passkey was added. Try again, or use an authenticator app instead.';
      } else {
        error = e instanceof DOMException ? 'No passkey was added. Use an authenticator app instead.' : messageOf(e);
      }
    } finally {
      busy = false;
    }
  }
</script>

<section class="{card} mx-auto mt-16 max-w-md">
  {#if step === 'app'}
    <AppSetup onDone={added} onCancel={() => (step = 'choose')} onLogin={onDone} />
  {:else if step === 'codes'}
    <RecoveryCodes {codes} {onDone} />
  {:else}
    <h1 class="text-2xl font-semibold tracking-[-0.02em]">Protect your admin</h1>
    <p class="mt-1 text-muted">
      Add a second step at sign-in, so your password alone is not enough to get in. It takes a minute, and we
      strongly suggest it.
    </p>
    <button class="{primary} mt-6 w-full" type="button" disabled={busy} onclick={() => (step = 'app')}>Use an authenticator app</button>
    <p class="mt-1.5 text-sm text-muted">A 6-digit code from an app on your phone each time you sign in.</p>
    {#if passkeys}
      <button class="{button} mt-4 w-full" type="button" disabled={busy} onclick={passkey}>Add a passkey</button>
      <p class="mt-1.5 text-sm text-muted">Sign in with your fingerprint, face or device PIN instead of the password.</p>
    {/if}
    <p class="mt-4 text-sm font-semibold" role="status">{note}</p>
    <p class="mt-1 font-semibold text-bad" role="alert">{error}</p>
    <button class="mt-4 w-full rounded-lg py-1 text-sm font-semibold text-accent hover:underline {focusRing}" type="button" disabled={busy} onclick={onDone}>Skip for now</button>
    <p class="mt-1 text-center text-sm text-muted">You can turn it on later, under Security.</p>
  {/if}
</section>
