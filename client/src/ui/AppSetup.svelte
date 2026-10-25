<script lang="ts">
  // Adds easi.run to an authenticator app: its QR code, or the key to type, then a code from the app, which turns it
  // on. In place of an app that is on (`replacing`), it also takes the current password.
  import { api, messageOf, statusOf } from './api.ts';
  import Qr from './Qr.svelte';
  import { appCode } from './second.ts';
  import { button, field, label, primary } from './ui.ts';

  let {
    replacing = false,
    onDone,
    onCancel,
    onLogin,
  }: {
    replacing?: boolean;
    /** Turned on: the recovery codes, when this turned the second step on. */
    onDone: (codes: string[] | null) => void;
    onCancel: () => void;
    /** The login ended meanwhile. */
    onLogin: () => void;
  } = $props();
  let drawn = $state<{ secret: string; uri: string } | null>(null);
  let code = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);
  let copied = $state(false);
  /** The key in groups of four, as apps that take it typed show it. */
  const key = $derived(drawn?.secret.match(/.{1,4}/g)?.join(' ') ?? '');

  /** Why a request failed, or the login, when that ended. */
  function failed(e: unknown): void {
    if (statusOf(e) === 401) return onLogin();
    error = messageOf(e);
  }

  function draw(): void {
    error = '';
    api<{ secret: string; uri: string }>('POST', '/api/security/app').then((answer) => (drawn = answer), failed);
  }
  $effect(draw);

  async function copyKey() {
    try {
      await navigator.clipboard.writeText(drawn?.secret ?? '');
      copied = true;
    } catch {
      error = 'Copying was refused; select the key and copy it yourself.';
    }
  }

  async function confirm(event: SubmitEvent) {
    event.preventDefault();
    const digits = appCode(code);
    if (!digits) {
      error = 'Enter the 6 digits the app shows.';
      return;
    }
    busy = true;
    error = '';
    try {
      const body = replacing ? { code: digits, password } : { code: digits };
      onDone((await api<{ recovery_codes: string[] | null }>('POST', '/api/security/app/confirm', body)).recovery_codes);
    } catch (e) {
      failed(e);
    } finally {
      busy = false;
    }
  }
</script>

<form onsubmit={confirm}>
  <h2 class="text-xl font-semibold tracking-[-0.02em]">Add easi.run to your authenticator app</h2>
  <p class="mt-1 text-muted">
    Any app that shows 6-digit codes works, such as Google Authenticator, Microsoft Authenticator, 1Password or
    Bitwarden.
  </p>
  {#if drawn}
    <p class="mt-4 text-sm font-semibold">1. In the app, add an account and scan this code:</p>
    <div class="mt-2 flex justify-center rounded-xl border border-line bg-paper p-3">
      <Qr text={drawn.uri} label="The QR code that adds easi.run to your authenticator app" />
    </div>
    <p class="mt-3 text-sm text-muted">Can't scan it? Enter this key in the app instead:</p>
    <div class="mt-1 flex flex-wrap items-center gap-3">
      <code class="min-w-0 flex-1 select-all break-all font-mono text-[15px]">{key}</code>
      <button class={button} type="button" onclick={copyKey}>{copied ? 'Copied' : 'Copy'}</button>
    </div>
    <label class="{label} mt-5" for="app-code">2. Enter the 6-digit code the app shows</label>
    <input id="app-code" class={field} type="text" inputmode="numeric" autocomplete="one-time-code" maxlength="7" required bind:value={code} />
    {#if replacing}
      <label class={label} for="app-password">Your current password</label>
      <input id="app-password" class={field} type="password" autocomplete="current-password" required bind:value={password} />
    {/if}
    <div class="mt-6 flex gap-3">
      <button class="{primary} flex-1" type="submit" disabled={busy}>Turn on</button>
      <button class={button} type="button" disabled={busy} onclick={onCancel}>Cancel</button>
    </div>
  {:else if !error}
    <p class="mt-4 text-muted">Loading…</p>
  {:else}
    <div class="mt-4 flex gap-3">
      <button class={primary} type="button" onclick={draw}>Try again</button>
      <button class={button} type="button" onclick={onCancel}>Cancel</button>
    </div>
  {/if}
  <p class="mt-3 font-semibold text-bad" role="alert">{error}</p>
</form>
