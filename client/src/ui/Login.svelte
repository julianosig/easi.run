<script lang="ts">
  // The login: the password, then, with the second step on, a code from the authenticator app or a recovery code (or
  // a passkey). A passkey added to this admin signs in on its own, in place of both, when this browser can use it.
  import { api, bodyOf, messageOf, setCsrf, statusOf, TOO_MANY_ATTEMPTS, wrongOrigin } from './api.ts';
  import { appCode, passkeyRefusal, passkeysInBrowser, type PendingStep, signInWithPasskey } from './second.ts';
  import Command from './Command.svelte';
  import Gate from './Gate.svelte';
  import Icon from './Icon.svelte';
  import { button, codeField, field, focusRing, GATE_ICONS, gateLead, gatePrimary, gateTitle, label } from './ui.ts';
  import WrongAddress from './WrongAddress.svelte';

  let { passkey, onDone }: { passkey: boolean; onDone: () => void } = $props();
  let password = $state('');
  let code = $state('');
  /** The second step the password opened, while it waits for its code. */
  let pending = $state<PendingStep | null>(null);
  /** A recovery code is typed in place of the app's. */
  let recovery = $state(false);
  let error = $state('');
  let busy = $state(false);
  /** The address the admin is set up for, when it refused this one. */
  let elsewhere: string | null = $state(null);
  const offerPasskey = $derived(passkey && passkeysInBrowser());
  /** Only passkeys are on (no app): the passkey leads, a recovery code below it. */
  const passkeyFirst = $derived(pending !== null && !pending.app && pending.passkey && passkeysInBrowser());
  let passwordInput: HTMLInputElement | undefined = $state();
  let codeInput: HTMLInputElement | undefined = $state();
  $effect(() => (pending ? codeInput : passwordInput)?.focus());

  /** What a refused request means to the person. */
  function why(e: unknown, wrong: string): string {
    if (statusOf(e) === 429) return TOO_MANY_ATTEMPTS;
    if (statusOf(e) === 401) return wrong;
    return messageOf(e);
  }

  async function signIn(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = '';
    try {
      const answer = await api<{ csrf?: string; second_step?: PendingStep }>('POST', '/api/login', { password });
      if (answer.second_step) {
        pending = answer.second_step;
        recovery = !pending.app;
        password = '';
      } else {
        setCsrf(answer.csrf ?? '');
        onDone();
      }
    } catch (e) {
      elsewhere = wrongOrigin(e);
      error = why(e, 'That password is not right.');
    } finally {
      busy = false;
    }
  }

  async function answer(event: SubmitEvent) {
    event.preventDefault();
    if (!pending) return;
    const typed = recovery ? code.trim() : appCode(code);
    if (!typed) {
      error = 'Enter the 6 digits the app shows.';
      return;
    }
    busy = true;
    error = '';
    try {
      setCsrf((await api<{ csrf: string }>('POST', '/api/login/code', { step: pending.step, code: typed })).csrf);
      onDone();
    } catch (e) {
      code = '';
      if (statusOf(e) === 401 && bodyOf(e).reason === 'again') {
        pending = null;
        error = 'That took too long. Enter your password again.';
      } else {
        error = why(e, recovery ? 'That is not one of your recovery codes.' : 'That code is not right. Try the one the app shows now.');
      }
    } finally {
      busy = false;
    }
  }

  async function withPasskey() {
    busy = true;
    error = '';
    try {
      setCsrf(await signInWithPasskey());
      onDone();
    } catch (e) {
      const refused = passkeyRefusal(e);
      if (refused === 'untrusted') {
        error = 'This browser offers passkeys only on a site with a certificate it trusts. Use your password here.';
      } else if (refused === 'cancelled') {
        error = 'The passkey did not sign in. Try again, or use your password.';
      } else {
        error = why(e, 'That passkey did not sign in. Try again, or use your password.');
      }
    } finally {
      busy = false;
    }
  }

  function switchCode() {
    recovery = !recovery;
    code = '';
    error = '';
    codeInput?.focus();
  }

  const RESET = 'docker exec easi-run-admin-1 admin reset-second-step';
  const link = `text-sm font-semibold text-accent hover:underline rounded ${focusRing}`;
</script>

{#snippet title(text: string)}
  <h1 class={gateTitle}>{text}</h1>
{/snippet}

<Gate label={pending ? 'Second step' : 'Welcome back'}>
  {#if pending}
    {#if passkeyFirst}
      {@render title("Confirm it's you")}
      <p class={gateLead}>Use the passkey you added to this admin.</p>
      <button class="{gatePrimary} mt-7 flex w-full items-center justify-center gap-2" type="button" disabled={busy} onclick={withPasskey}
        ><Icon d={GATE_ICONS.passkey} />Use your passkey</button
      >
      <p class="mt-8 text-sm text-muted">Lost it? Enter one of the recovery codes you saved instead.</p>
    {/if}
    <form onsubmit={answer}>
      {#if !passkeyFirst}
        {@render title(recovery ? 'Enter a recovery code' : 'Enter your code')}
        <p class={gateLead}>
          {recovery
            ? 'Enter one of the recovery codes you saved. Each one works once.'
            : 'Open your authenticator app and enter the 6-digit code it shows for easi.run.'}
        </p>
      {/if}
      <label class={label} for="code">{recovery ? 'Recovery code' : 'Code'}</label>
      {#if recovery}
        <input id="code" class="{field} font-mono" type="text" autocomplete="off" spellcheck="false" placeholder="xxxx-xxxx-xxxx-xxxx" required bind:value={code} bind:this={codeInput} />
      {:else}
        <input id="code" class={codeField} type="text" inputmode="numeric" autocomplete="one-time-code" maxlength="7" required bind:value={code} bind:this={codeInput} />
      {/if}
      <button class="{passkeyFirst ? button : gatePrimary} mt-7 w-full" type="submit" disabled={busy}>Log in</button>
      <p class="mt-3 font-semibold text-bad" role="alert">{error}</p>
      <div class="mt-2 flex flex-col items-start gap-2">
        {#if pending.app}
          <button class={link} type="button" onclick={switchCode}>{recovery ? 'Use the authenticator app instead' : 'Use a recovery code instead'}</button>
        {/if}
        {#if pending.passkey && passkeysInBrowser() && !passkeyFirst}
          <button class={link} type="button" disabled={busy} onclick={withPasskey}>Use your passkey instead</button>
        {/if}
      </div>
      <div class="mt-10">
        <p class="text-sm text-muted">Lost your phone or passkey and your recovery codes? On the server, this turns the second step off:</p>
        <Command command={RESET} where="On the server" name="the command that turns the second step off" />
      </div>
    </form>
  {:else}
    <form onsubmit={signIn}>
      {@render title('Log in')}
      <p class={gateLead}>Enter the password you created for this admin.</p>
      <label class={label} for="password">Password</label>
      <input id="password" class={field} type="password" autocomplete="current-password" required bind:value={password} bind:this={passwordInput} />
      <button class="{gatePrimary} mt-7 w-full" type="submit" disabled={busy}>Log in</button>
    </form>
    {#if offerPasskey}
      <p class="mb-2.5 mt-7 text-center text-[13px] font-medium text-muted">Added a passkey? It signs you in without the password.</p>
      <button class="{button} flex w-full items-center justify-center gap-2" type="button" disabled={busy} onclick={withPasskey}
        ><Icon d={GATE_ICONS.passkey} class="size-[18px] text-accent" />Sign in with a passkey</button
      >
    {/if}
    {#if elsewhere}
      <WrongAddress origin={elsewhere} href={`${elsewhere}${location.pathname}`} />
    {:else}
      <p class="mt-3 font-semibold text-bad" role="alert">{error}</p>
    {/if}
  {/if}
</Gate>
