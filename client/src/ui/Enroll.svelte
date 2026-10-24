<script lang="ts">
  // The admin's first screen: the person creates the password with the token of the setup link that opened it, or of
  // one pasted here, and is logged in at once.
  import { api, messageOf, setCsrf, statusOf, type Status } from './api.ts';
  import { checkPassword, MAX_PASSWORD, MIN_PASSWORD, setupToken } from './setup.ts';
  import { card, field, label, primary } from './ui.ts';

  let {
    token: linked,
    onDone,
    onLogin,
  }: { token: string | null; onDone: () => void; onLogin: () => void } = $props();
  const NEW_LINK = 'docker exec -i easi-run-admin-1 admin setup-link';
  let link = $state('');
  let password = $state('');
  let confirm = $state('');
  let shown = $state(false);
  let error = $state('');
  let busy = $state(false);
  /** The admin refused the link's token (used, or expired): one pasted here takes its place. */
  let refused = $state(false);
  const fromLink = $derived(linked !== null && !refused);
  const token = $derived(fromLink ? linked : setupToken(link));
  const check = $derived(checkPassword(password, confirm));
  let linkInput: HTMLInputElement | undefined = $state();
  let passwordInput: HTMLInputElement | undefined = $state();
  // The first field left to fill: the password's, or the link's when there is no link's token to use.
  $effect(() => (fromLink ? passwordInput : linkInput)?.focus());

  const problems = {
    short: `Use at least ${MIN_PASSWORD} characters.`,
    long: `Use at most ${MAX_PASSWORD} characters.`,
    different: 'The two passwords do not match.',
  };

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!token) {
      error = link.trim() ? 'That is not a setup link: paste the whole link the installer printed.' : 'Paste the setup link first.';
      return;
    }
    if (check.problem) {
      error = problems[check.problem];
      return;
    }
    error = '';
    busy = true;
    try {
      setCsrf((await api<{ csrf: string }>('POST', '/api/enroll', { token, password })).csrf);
      onDone();
    } catch (e) {
      if (statusOf(e) !== 403) {
        error = messageOf(e);
      } else if ((await api<Status>('GET', '/api/status').catch(() => null))?.enrolled) {
        // The password was set meanwhile (in another tab, or on the server): it logs in from now on.
        onLogin();
      } else {
        refused = true;
        link = '';
        error = 'This setup link has expired or was used already: get a new one as shown above.';
      }
    } finally {
      busy = false;
    }
  }
</script>

<form class="{card} mx-auto mt-16 max-w-md" onsubmit={submit}>
  <h1 class="text-2xl font-semibold tracking-[-0.02em]">Create your password</h1>
  {#if fromLink}
    <p class="mt-1 text-muted">You will log in to easi.run with it from now on.</p>
  {:else}
    <p class="mt-1 text-muted">
      Open the setup link the installer printed. Lost it? On the server: <code class="break-all font-mono text-sm">{NEW_LINK}</code>
    </p>
    <label class={label} for="link">Or paste the setup link here</label>
    <input id="link" class={field} type="text" autocomplete="off" spellcheck="false" placeholder="https://localhost:9443/#setup=…" bind:value={link} bind:this={linkInput} />
  {/if}
  <label class={label} for="password">Password</label>
  <input id="password" class={field} type={shown ? 'text' : 'password'} autocomplete="new-password" aria-describedby="password-hint" bind:value={password} bind:this={passwordInput} />
  <p id="password-hint" class="mt-1.5 text-sm {check.missing ? 'text-muted' : 'text-good'}">
    {check.missing ? `${MIN_PASSWORD} characters or more: ${check.missing} to go` : `${MIN_PASSWORD} characters or more ✓`}
  </p>
  <label class={label} for="confirm">Confirm password</label>
  <input id="confirm" class={field} type={shown ? 'text' : 'password'} autocomplete="new-password" aria-invalid={check.differs} aria-describedby="confirm-hint" bind:value={confirm} />
  <p id="confirm-hint" class="mt-1.5 text-sm font-semibold text-bad">{check.differs ? problems.different : ''}</p>
  <label class="mt-2 flex items-center gap-2 text-sm">
    <input class="accent-accent" type="checkbox" bind:checked={shown} /> Show the password
  </label>
  <button class="{primary} mt-6 w-full" type="submit" disabled={busy}>Create password</button>
  <p class="mt-3 font-semibold text-bad" role="alert">{error}</p>
</form>
