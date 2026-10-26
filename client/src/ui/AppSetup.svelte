<script lang="ts">
  // Adds easi.run to an authenticator app: its QR code, or the key to type, then a code from the app, which turns it
  // on. In place of an app that is on (`replacing`), it also takes the current password.
  import { api, copyText } from './api.ts';
  import { requests } from './requests.svelte.ts';
  import Qr from './Qr.svelte';
  import { appCode } from './second.ts';
  import { button, codeField, field, focusRing, label, primary, stepTitle } from './ui.ts';
  import Skeleton from './Skeleton.svelte';

  let {
    replacing = false,
    heading = stepTitle,
    onDone,
    onCancel,
    onLogin,
  }: {
    replacing?: boolean;
    /** Its headline's classes, larger on the screens before a login. */
    heading?: string;
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

  const page = requests(() => onLogin());
  /** Why a request failed, or the login, when that ended. */
  const failed = (e: unknown) => page.fail(e, (text) => (error = text));

  function draw(): void {
    error = '';
    api<{ secret: string; uri: string }>('POST', '/api/security/app').then((answer) => (drawn = answer), failed);
  }
  $effect(draw);

  async function copyKey() {
    if (await copyText(drawn?.secret ?? '')) copied = true;
    else error = 'Copying was refused; select the key and copy it yourself.';
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

{#snippet number(n: number)}
  <span class="grid size-6 shrink-0 place-items-center rounded-full bg-accent text-xs font-bold text-white">{n}</span>
{/snippet}

<form onsubmit={confirm}>
  <h2 class={heading}>Add easi.run to your authenticator app</h2>
  <p class="mt-1 text-muted">
    Any app that shows 6-digit codes works, such as Google Authenticator, Microsoft Authenticator, 1Password or
    Bitwarden.
  </p>
  {#if drawn}
    <p class="mt-5 flex items-center gap-2.5 text-sm font-semibold">{@render number(1)}In the app, add an account and scan this code:</p>
    <!-- The code straight on the page, left-aligned with the steps: only its own white quiet zone around it, for
         scanners, on whatever ground. -->
    <div class="mt-3">
      <div class="-ml-2.5 w-56 rounded-lg bg-white p-2.5">
        <Qr text={drawn.uri} label="The QR code that adds easi.run to your authenticator app" />
      </div>
      <div>
        <p class="mt-3 text-sm text-muted">Can't scan it? Enter this key in the app instead:</p>
        <div class="mt-1.5 flex items-center gap-2 rounded-xl bg-wash py-1.5 pl-3.5 pr-1.5">
          <code class="min-w-0 flex-1 select-all font-mono text-[14px] tracking-wide">{key}</code>
          <button class="shrink-0 rounded-lg border border-line bg-paper px-3 py-1.5 text-sm font-semibold text-ink transition hover:border-ink/30 {focusRing}" type="button" onclick={copyKey}>{copied ? 'Copied' : 'Copy'}</button>
        </div>
      </div>
    </div>
    <label class="mb-1.5 mt-6 flex items-center gap-2.5 text-sm font-semibold" for="app-code">{@render number(2)}Enter the 6-digit code the app shows</label>
    <input id="app-code" class={codeField} type="text" inputmode="numeric" autocomplete="one-time-code" maxlength="7" required bind:value={code} />
    {#if replacing}
      <label class={label} for="app-password">Your current password</label>
      <input id="app-password" class={field} type="password" autocomplete="current-password" required bind:value={password} />
    {/if}
    <div class="mt-6 flex gap-3">
      <button class="{primary} flex-1" type="submit" disabled={busy}>Turn on</button>
      <button class={button} type="button" disabled={busy} onclick={onCancel}>Cancel</button>
    </div>
  {:else if !error}
    <Skeleton said="Preparing your code" rows={1} />
  {:else}
    <div class="mt-4 flex gap-3">
      <button class={primary} type="button" onclick={draw}>Try again</button>
      <button class={button} type="button" onclick={onCancel}>Cancel</button>
    </div>
  {/if}
  <p class="mt-3 font-semibold text-bad" role="alert">{error}</p>
</form>
