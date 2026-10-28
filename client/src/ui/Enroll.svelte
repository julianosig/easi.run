<script lang="ts">
  // The admin's first screen, while no password is set: the person creates it here and is logged in at once; the
  // password goes on to the next screen, which offers a second step.
  import { type Said, t } from '../i18n/index.svelte.ts';
  import { api, messageOf, setCsrf, type Status, statusOf, wrongOrigin } from './api.ts';
  import { checkPassword, MAX_PASSWORD, MIN_PASSWORD } from './setup.ts';
  import Gate from './Gate.svelte';
  import LanguagePicker from './LanguagePicker.svelte';
  import { field, gateLead, gatePrimary, gateTitle, label } from './ui.ts';
  import WrongAddress from './WrongAddress.svelte';

  let { onDone, onLogin }: { onDone: (password: string) => void; onLogin: () => void } = $props();
  let password = $state('');
  let confirm = $state('');
  let shown = $state(false);
  let error: Said | null = $state(null);
  let busy = $state(false);
  /** The address the admin is set up for, when it refused this one. */
  let elsewhere: string | null = $state(null);
  const check = $derived(checkPassword(password, confirm));
  let passwordInput: HTMLInputElement | undefined = $state();
  $effect(() => passwordInput?.focus());

  const problems = {
    short: () => t('account.enroll.short', { count: MIN_PASSWORD }),
    long: () => t('account.enroll.long', { count: MAX_PASSWORD }),
    different: () => t('account.enroll.different'),
  };

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (check.problem) {
      error = problems[check.problem];
      return;
    }
    error = null;
    elsewhere = null;
    busy = true;
    try {
      setCsrf((await api<{ csrf: string }>('POST', '/api/enroll', { password })).csrf);
      onDone(password);
    } catch (e) {
      elsewhere = wrongOrigin(e);
      if (!elsewhere && statusOf(e) === 403) {
        // A password was set meanwhile (in another tab, or on the server): it logs in from now on.
        const status = await api<Status>('GET', '/api/status').catch(() => null);
        if (status?.enrolled) onLogin();
        else error = () => t('account.enroll.closed');
      } else {
        const message = messageOf(e);
        error = () => message;
      }
    } finally {
      busy = false;
    }
  }
</script>

{#snippet title(text: string)}
  <h1 class={gateTitle}>{text}</h1>
{/snippet}

<Gate label={t('account.enroll.label')}>
  <form onsubmit={submit}>
    {@render title(t('account.enroll.title'))}
    <p class={gateLead}>{t('account.enroll.lead')}</p>
    <label class={label} for="password">{t('account.password')}</label>
    <input id="password" class={field} type={shown ? 'text' : 'password'} autocomplete="new-password" aria-describedby="password-hint" bind:value={password} bind:this={passwordInput} />
    <p id="password-hint" class="mt-1.5 text-sm {check.missing ? 'text-muted' : 'text-good'}">
      {check.missing ? t('account.enroll.missing', { min: MIN_PASSWORD, count: check.missing }) : t('account.enroll.enough', { count: MIN_PASSWORD })}
    </p>
    <label class={label} for="confirm">{t('account.enroll.confirm')}</label>
    <input id="confirm" class={field} type={shown ? 'text' : 'password'} autocomplete="new-password" aria-invalid={check.differs} aria-describedby="confirm-hint" bind:value={confirm} />
    <p id="confirm-hint" class="mt-1.5 text-sm font-semibold text-bad">{check.differs ? problems.different() : ''}</p>
    <label class="mt-2 flex items-center gap-2 text-sm">
      <input class="accent-accent" type="checkbox" bind:checked={shown} /> {t('account.enroll.show')}
    </label>
    <button class="{gatePrimary} mt-7 w-full" type="submit" disabled={busy}>{t('account.enroll.submit')}</button>
    {#if elsewhere}
      <WrongAddress origin={elsewhere} href={`${elsewhere}${location.pathname}`} />
    {:else}
      <p class="mt-3 font-semibold text-bad" role="alert">{error?.()}</p>
    {/if}
  </form>
  <div class="mt-10 flex justify-center"><LanguagePicker /></div>
</Gate>
