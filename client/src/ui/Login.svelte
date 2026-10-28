<script lang="ts">
  // The login: the password, then, with the second step on, a code from the authenticator app or a recovery code (or
  // a passkey). A passkey added to this admin signs in on its own, in place of both, when this browser can use it.
  import { type Said, t } from '../i18n/index.svelte.ts';
  import { api, bodyOf, messageOf, setCsrf, statusOf, TOO_MANY_ATTEMPTS, wrongOrigin } from './api.ts';
  import { appCode, passkeyRefusal, passkeysInBrowser, type PendingStep, signInWithPasskey } from './second.ts';
  import Command from './Command.svelte';
  import Gate from './Gate.svelte';
  import Icon from './Icon.svelte';
  import LanguagePicker from './LanguagePicker.svelte';
  import { button, codeField, field, GATE_ICONS, gateLead, gatePrimary, gateTitle, label, textLink } from './ui.ts';
  import WrongAddress from './WrongAddress.svelte';

  let { passkey, onDone }: { passkey: boolean; onDone: () => void } = $props();
  let password = $state('');
  let code = $state('');
  /** The second step the password opened, while it waits for its code. */
  let pending = $state<PendingStep | null>(null);
  /** A recovery code is typed in place of the app's. */
  let recovery = $state(false);
  let error: Said | null = $state(null);
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
  function why(e: unknown, wrong: Said): Said {
    if (statusOf(e) === 429) return () => t(TOO_MANY_ATTEMPTS);
    if (statusOf(e) === 401) return wrong;
    const message = messageOf(e);
    return () => message;
  }

  async function signIn(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = null;
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
      error = why(e, () => t('account.login.wrong'));
    } finally {
      busy = false;
    }
  }

  async function answer(event: SubmitEvent) {
    event.preventDefault();
    if (!pending) return;
    const typed = recovery ? code.trim() : appCode(code);
    if (!typed) {
      error = () => t('account.code.digits');
      return;
    }
    busy = true;
    error = null;
    try {
      setCsrf((await api<{ csrf: string }>('POST', '/api/login/code', { step: pending.step, code: typed })).csrf);
      onDone();
    } catch (e) {
      code = '';
      if (statusOf(e) === 401 && bodyOf(e).reason === 'again') {
        pending = null;
        error = () => t('account.login.again');
      } else {
        error = why(e, recovery ? () => t('account.second.recovery.wrong') : () => t('account.second.app.wrong'));
      }
    } finally {
      busy = false;
    }
  }

  async function withPasskey() {
    busy = true;
    error = null;
    try {
      setCsrf(await signInWithPasskey());
      onDone();
    } catch (e) {
      const refused = passkeyRefusal(e);
      if (refused === 'untrusted') {
        error = () => t('account.login.passkey.untrusted');
      } else if (refused === 'cancelled') {
        error = () => t('account.login.passkey.cancelled');
      } else {
        error = why(e, () => t('account.login.passkey.refused'));
      }
    } finally {
      busy = false;
    }
  }

  function switchCode() {
    recovery = !recovery;
    code = '';
    error = null;
    codeInput?.focus();
  }

  const RESET = 'docker exec easi-run-admin-1 admin reset-second-step';
  const link = textLink;
</script>

{#snippet title(text: string)}
  <h1 class={gateTitle}>{text}</h1>
{/snippet}

<Gate label={pending ? t('account.second.label') : t('account.login.label')}>
  {#if pending}
    {#if passkeyFirst}
      {@render title(t('account.second.passkey.title'))}
      <p class={gateLead}>{t('account.second.passkey.lead')}</p>
      <button class="{gatePrimary} mt-7 flex w-full items-center justify-center gap-2" type="button" disabled={busy} onclick={withPasskey}
        ><Icon d={GATE_ICONS.passkey} />{t('account.second.passkey.use')}</button
      >
      <p class="mt-8 text-sm text-muted">{t('account.second.passkey.lost')}</p>
    {/if}
    <form onsubmit={answer}>
      {#if !passkeyFirst}
        {@render title(recovery ? t('account.second.recovery.title') : t('account.second.app.title'))}
        <p class={gateLead}>
          {recovery ? t('account.second.recovery.lead') : t('account.second.app.lead')}
        </p>
      {/if}
      <label class={label} for="code">{recovery ? t('account.second.recovery.label') : t('account.second.app.label')}</label>
      {#if recovery}
        <input id="code" class="{field} font-mono" type="text" autocomplete="off" spellcheck="false" placeholder="xxxx-xxxx-xxxx-xxxx" required bind:value={code} bind:this={codeInput} />
      {:else}
        <input id="code" class={codeField} type="text" inputmode="numeric" autocomplete="one-time-code" maxlength="7" required bind:value={code} bind:this={codeInput} />
      {/if}
      <button class="{passkeyFirst ? button : gatePrimary} mt-7 w-full" type="submit" disabled={busy}>{t('account.login.submit')}</button>
      <p class="mt-3 font-semibold text-bad" role="alert">{error?.()}</p>
      <div class="mt-2 flex flex-col items-start gap-2">
        {#if pending.app}
          <button class={link} type="button" onclick={switchCode}>{recovery ? t('account.second.app.switch') : t('account.second.recovery.switch')}</button>
        {/if}
        {#if pending.passkey && passkeysInBrowser() && !passkeyFirst}
          <button class={link} type="button" disabled={busy} onclick={withPasskey}>{t('account.second.passkey.switch')}</button>
        {/if}
      </div>
      <div class="mt-10">
        <p class="text-sm text-muted">{t('account.second.reset.lead')}</p>
        <Command command={RESET} where={t('account.second.reset.where')} name={t('account.second.reset.name')} />
      </div>
    </form>
  {:else}
    <form onsubmit={signIn}>
      {@render title(t('account.login.title'))}
      <p class={gateLead}>{t('account.login.lead')}</p>
      <label class={label} for="password">{t('account.password')}</label>
      <input id="password" class={field} type="password" autocomplete="current-password" required bind:value={password} bind:this={passwordInput} />
      <button class="{gatePrimary} mt-7 w-full" type="submit" disabled={busy}>{t('account.login.submit')}</button>
    </form>
    {#if offerPasskey}
      <p class="mb-2.5 mt-7 text-center text-[13px] font-medium text-muted">{t('account.login.passkey.lead')}</p>
      <button class="{button} flex w-full items-center justify-center gap-2" type="button" disabled={busy} onclick={withPasskey}
        ><Icon d={GATE_ICONS.passkey} class="size-[18px] text-accent" />{t('account.login.passkey')}</button
      >
    {/if}
    {#if elsewhere}
      <WrongAddress origin={elsewhere} href={`${elsewhere}${location.pathname}`} />
    {:else}
      <p class="mt-3 font-semibold text-bad" role="alert">{error?.()}</p>
    {/if}
  {/if}
  <div class="mt-10 flex justify-center"><LanguagePicker /></div>
</Gate>
