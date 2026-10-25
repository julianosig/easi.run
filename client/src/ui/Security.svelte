<script lang="ts">
  // Security, after login: the second step at sign-in, to turn on, off, or replace. Removing anything, replacing the
  // app, adding a passkey and new recovery codes ask for the current password again; every change signs out every
  // other browser.
  import { api, bodyOf, messageOf, statusOf } from './api.ts';
  import AppSetup from './AppSetup.svelte';
  import PasswordCheck from './PasswordCheck.svelte';
  import RecoveryCodes from './RecoveryCodes.svelte';
  import { addPasskey, type PasskeyInfo, passkeyRefusal, passkeysInBrowser, type SecondStep, UNTRUSTED_PASSKEY } from './second.ts';
  import { button, card, panelTitle, primary } from './ui.ts';

  let { onLogin }: { onLogin: () => void } = $props();
  let status = $state<SecondStep | null>(null);
  /** What is shown in place of the overview: the app being added or replaced, or recovery codes to save. */
  let showing: 'overview' | 'app' | 'replace' | 'codes' = $state('overview');
  let codes: string[] = $state([]);
  /** The change waiting for the current password. */
  let asking = $state<{ title: string; action: string; run: (password: string) => Promise<void> } | null>(null);
  let error = $state('');
  let note = $state('');
  const passkeysHere = passkeysInBrowser();
  const on = $derived(status !== null && (status.app || status.passkeys.length > 0));

  async function load() {
    try {
      status = await api<SecondStep>('GET', '/api/security');
    } catch (e) {
      if (statusOf(e) === 401) return onLogin();
      error = messageOf(e);
    }
  }
  $effect(() => {
    load();
  });

  /** Shows the recovery codes a change issued, if any; the overview again otherwise. */
  function changed(issued: string[] | null) {
    asking = null;
    if (issued) {
      codes = issued;
      showing = 'codes';
    } else {
      showing = 'overview';
    }
    load();
  }

  /** Why a change failed, as the person reads it. */
  function explain(e: unknown): string {
    if (statusOf(e) === 401) {
      onLogin();
      return '';
    }
    if (bodyOf(e).reason === 'password') return 'That is not your current password.';
    if (statusOf(e) === 429) return 'Too many attempts. Wait a minute, then try again.';
    const refused = passkeyRefusal(e);
    if (refused === 'untrusted') return UNTRUSTED_PASSKEY;
    if (refused === 'cancelled') return 'No passkey was added. Try again when you are ready.';
    if (refused === 'added') return 'This device holds a passkey for this admin already.';
    return messageOf(e);
  }

  function ask(title: string, action: string, run: (password: string) => Promise<void>) {
    note = '';
    asking = { title, action, run };
  }

  async function confirmed(password: string): Promise<string | null> {
    try {
      await asking?.run(password);
      return null;
    } catch (e) {
      return explain(e);
    }
  }

  /** What removing one factor leaves: with nothing else on, the password alone signs in, and the codes go. */
  const last = (others: number) =>
    others === 0 ? ' Nothing else is on: your password alone will sign in, and your recovery codes stop working.' : '';
  const turnOffApp = () =>
    ask(`Turn off the authenticator app?${last(status?.passkeys.length ?? 0)}`, 'Turn off', async (password) => {
      await api('POST', '/api/security/app/remove', { password });
      changed(null);
      note = 'The authenticator app is off.';
    });
  const newPasskey = () =>
    ask('Add a passkey on this device', 'Continue', async (password) => changed(await addPasskey(password)));
  const removePasskey = (passkey: PasskeyInfo) => {
    const others = (status?.passkeys.length ?? 1) - 1 + (status?.app ? 1 : 0);
    ask(`Remove the passkey added ${added(passkey.created)}?${last(others)}`, 'Remove', async (password) => {
      await api('POST', `/api/security/passkeys/${passkey.id}/remove`, { password });
      changed(null);
      note = 'The passkey is removed.';
    });
  };
  const newCodes = () =>
    ask('Get new recovery codes? The ones you have stop working.', 'Get new codes', async (password) =>
      changed((await api<{ recovery_codes: string[] }>('POST', '/api/security/recovery', { password })).recovery_codes),
    );
  /** When a passkey was added, to the second, which tells two apart: adding one takes longer than that. */
  const added = (seconds: number) =>
    new Date(seconds * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'medium' });
</script>

<section class="{card} mx-auto mt-8 max-w-xl">
  {#if showing === 'app' || showing === 'replace'}
    <AppSetup replacing={showing === 'replace'} onDone={changed} onCancel={() => (showing = 'overview')} {onLogin} />
  {:else if showing === 'codes'}
    <RecoveryCodes {codes} onDone={() => (showing = 'overview')} />
  {:else}
    <h1 class="text-2xl font-semibold tracking-[-0.02em]">Security</h1>
    <p class="mt-1 text-muted">
      A second step at sign-in keeps your admin yours even if someone learns your password. Any change here signs out
      every other browser.
    </p>
    {#if status === null}
      <p class="mt-6 text-muted">{error ? '' : 'Loading…'}</p>
    {:else}
      {#if !on}
        <p class="mt-4 rounded-xl border border-you/40 bg-you/5 p-3 text-sm font-semibold">
          Your admin signs in with the password alone. Turn on a second step to protect it.
        </p>
      {/if}

      <h2 class="{panelTitle} mt-6">Authenticator app</h2>
      <p class="text-sm text-muted">{status.app ? 'On: sign-ins ask for a code from the app.' : 'Off.'}</p>
      <div class="mt-3 flex flex-wrap gap-3">
        {#if status.app}
          <button class={button} type="button" onclick={() => ((showing = 'replace'), (asking = null))}>Replace</button>
          <button class={button} type="button" onclick={turnOffApp}>Turn off</button>
        {:else}
          <button class={primary} type="button" onclick={() => ((showing = 'app'), (asking = null))}>Turn on</button>
        {/if}
      </div>

      <h2 class="{panelTitle} mt-8">Passkeys</h2>
      {#if status.passkeys.length > 0}
        <ul class="divide-y divide-line">
          {#each status.passkeys as passkey (passkey.id)}
            <li class="flex items-center gap-3 py-2.5">
              <span class="flex-1">Passkey<span class="text-sm text-muted"> · added {added(passkey.created)}</span></span>
              <button class={button} type="button" onclick={() => removePasskey(passkey)}>Remove</button>
            </li>
          {/each}
        </ul>
      {/if}
      {#if !status.passkeys_possible}
        <p class="text-sm text-muted">Passkeys need this admin at a domain name, such as localhost, not an IP address.</p>
      {:else if !passkeysHere}
        <p class="text-sm text-muted">This browser offers no passkeys on this page. Use an authenticator app instead.</p>
      {:else}
        <p class="text-sm text-muted">A passkey signs you in on its own, with your fingerprint, face or device PIN.</p>
        <button class="{button} mt-3" type="button" onclick={newPasskey}>Add a passkey</button>
      {/if}

      {#if on}
        <h2 class="{panelTitle} mt-8">Recovery codes</h2>
        <p class="text-sm text-muted">
          {status.recovery_codes === 1 ? '1 code left' : `${status.recovery_codes} codes left`}: if you lose your phone or
          passkey, sign in with your password, then enter one; each works once.
        </p>
        <button class="{button} mt-3" type="button" onclick={newCodes}>Get new codes</button>
      {/if}

      {#if asking}
        {#key asking}
          <PasswordCheck title={asking.title} action={asking.action} onConfirm={confirmed} onCancel={() => (asking = null)} />
        {/key}
      {/if}
      <p class="mt-4 text-sm font-semibold text-good" role="status">{note}</p>
    {/if}
    <p class="mt-3 font-semibold text-bad" role="alert">{error}</p>
  {/if}
</section>
