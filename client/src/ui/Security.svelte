<script lang="ts">
  // Security, after login: the second step at sign-in, to turn on, off, or replace. Removing anything, replacing the
  // app, adding a passkey and new recovery codes ask for the current password again; every change signs out every
  // other browser.
  import { api, bodyOf, messageOf, statusOf } from './api.ts';
  import AppSetup from './AppSetup.svelte';
  import PasswordCheck from './PasswordCheck.svelte';
  import RecoveryCodes from './RecoveryCodes.svelte';
  import { addPasskey, type PasskeyInfo, passkeyRefusal, passkeysInBrowser, type SecondStep, UNTRUSTED_PASSKEY } from './second.ts';
  import type { Snippet } from 'svelte';
  import { CHIP_TONES, chip, FORM_ICONS, PAGE_ICONS, PLUS_ICON, row, rowAction, rowPrimary } from './admin.ts';
  import PageHeader from './PageHeader.svelte';
  import Icon from './Icon.svelte';
  import { GATE_ICONS } from './ui.ts';

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

{#snippet part(icon: string, title: string, state: { on: boolean; text: string } | null, line: string, actions: Snippet)}
  <section class="flex flex-wrap items-start gap-x-4 gap-y-3 rounded-2xl px-3 py-4 transition hover:bg-wash/60 sm:flex-nowrap">
    <span class="grid size-10 shrink-0 place-items-center rounded-xl bg-accent-soft text-accent"><Icon d={icon} class="size-5 shrink-0" /></span>
    <div class="min-w-0 flex-1 basis-56">
      <h2 class="flex flex-wrap items-center gap-2 text-[17px] font-semibold tracking-tight">
        {title}
        {#if state}<span class="{chip} {state.on ? CHIP_TONES.good : CHIP_TONES.neutral}"><span class="size-1.5 rounded-full {state.on ? 'bg-good' : 'bg-muted/50'}" aria-hidden="true"></span>{state.text}</span>{/if}
      </h2>
      <p class="mt-0.5 text-sm text-muted">{line}</p>
    </div>
    <div class="flex shrink-0 flex-wrap gap-2 max-sm:pl-14">{@render actions()}</div>
  </section>
{/snippet}

<div>
  {#if showing === 'app' || showing === 'replace'}
    <div class="max-w-xl"><AppSetup replacing={showing === 'replace'} onDone={changed} onCancel={() => (showing = 'overview')} {onLogin} /></div>
  {:else if showing === 'codes'}
    <div class="max-w-xl"><RecoveryCodes {codes} onDone={() => (showing = 'overview')} /></div>
  {:else}
    <PageHeader icon={PAGE_ICONS.security} title="Security" subtitle="A second step at sign-in keeps your admin yours, even if someone learns your password." />
    {#if status === null}
      <p class="mt-10 text-muted">{error ? '' : 'Loading…'}</p>
    {:else}
      {#if !on}
        <p class="mt-8 flex items-center gap-2.5 rounded-2xl bg-you/[0.07] px-4 py-3 text-sm font-semibold">
          <Icon d={GATE_ICONS.warning} class="size-4 shrink-0 text-[#b5452b]" />Your admin signs in with the password alone. Turn on a second step to protect it.
        </p>
      {/if}

      <div class="mt-8 -mx-3 space-y-2">
        {#snippet appActions()}
          {#if status?.app}
            <button class={rowAction} type="button" onclick={() => ((showing = 'replace'), (asking = null))}>Replace</button>
            <button class={rowAction} type="button" onclick={turnOffApp}>Turn off</button>
          {:else}
            <button class={rowPrimary} type="button" onclick={() => ((showing = 'app'), (asking = null))}>Turn on</button>
          {/if}
        {/snippet}
        {@render part(GATE_ICONS.phone, 'Authenticator app', { on: status.app, text: status.app ? 'On' : 'Off' }, 'A 6-digit code from an app on your phone at each sign-in.', appActions)}

        {#snippet passkeyActions()}
          {#if status?.passkeys_possible && passkeysHere}
            <button class={rowAction} type="button" onclick={newPasskey}><Icon d={PLUS_ICON} class="size-4 text-accent" />Add a passkey</button>
          {/if}
        {/snippet}
        {@render part(
          GATE_ICONS.passkey,
          'Passkeys',
          { on: status.passkeys.length > 0, text: status.passkeys.length === 0 ? 'None' : status.passkeys.length === 1 ? '1 added' : `${status.passkeys.length} added` },
          !status.passkeys_possible
            ? 'Passkeys need this admin at a domain name, such as localhost, not an IP address.'
            : !passkeysHere
              ? 'This browser offers no passkeys on this page. Use an authenticator app instead.'
              : 'Sign in with your fingerprint, face or device PIN.',
          passkeyActions,
        )}
        {#if status.passkeys.length > 0}
          <ul class="ml-[4.25rem] space-y-1">
            {#each status.passkeys as passkey (passkey.id)}
              <li class="flex items-center gap-3 py-1.5 {row}">
                <span class="min-w-0 flex-1 text-sm">Passkey<span class="text-muted"> · added {added(passkey.created)}</span></span>
                <button class={rowAction} type="button" onclick={() => removePasskey(passkey)}>Remove</button>
              </li>
            {/each}
          </ul>
        {/if}

        {#if on}
          {#snippet codeActions()}
            <button class={rowAction} type="button" onclick={newCodes}>Get new codes</button>
          {/snippet}
          {@render part(
            FORM_ICONS.key,
            'Recovery codes',
            { on: status.recovery_codes > 0, text: status.recovery_codes === 1 ? '1 left' : `${status.recovery_codes} left` },
            'Lost your phone or passkey? Sign in with your password and one code; each works once.',
            codeActions,
          )}
        {/if}
      </div>
      <p class="mt-4 text-[13px] text-muted">Any change here signs out every other browser.</p>

      {#if asking}
        {#key asking}
          <PasswordCheck title={asking.title} action={asking.action} onConfirm={confirmed} onCancel={() => (asking = null)} />
        {/key}
      {/if}
      <p class="mt-4 text-sm font-semibold text-good" role="status">{note}</p>
    {/if}
    <p class="mt-3 font-semibold text-bad" role="alert">{error}</p>
  {/if}
</div>
