<script lang="ts">
  // Security, after login: the second step at sign-in, to turn on, off, or replace. Removing anything, replacing the
  // app, adding a passkey and new recovery codes ask for the current password again; every change signs out every
  // other browser.
  import Button from './Button.svelte';
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { api, bodyOf, messageOf, NOT_YOUR_PASSWORD, statusOf, TOO_MANY_ATTEMPTS } from './api.ts';
  import { requests } from './requests.svelte.ts';
  import AppSetup from './AppSetup.svelte';
  import PasswordCheck from './PasswordCheck.svelte';
  import RecoveryCodes from './RecoveryCodes.svelte';
  import { addPasskey, type PasskeyInfo, passkeyRefusal, passkeysInBrowser, type SecondStep, UNTRUSTED_PASSKEY } from './second.ts';
  import type { Snippet } from 'svelte';
  import { CHIP_TONES, chip, row } from './admin.ts';
  import PageHeader from './PageHeader.svelte';
  import Icon from './Icon.svelte';
  import Skeleton from './Skeleton.svelte';

  let { onLogin }: { onLogin: () => void } = $props();
  let status = $state<SecondStep | null>(null);
  /** What is shown in place of the overview: the app being added or replaced, or recovery codes to save. */
  let showing: 'overview' | 'app' | 'replace' | 'codes' = $state('overview');
  let codes: string[] = $state([]);
  /** The change waiting for the current password: its title and its button's label as the page's language says them. */
  let asking = $state<{ title: () => string; action: () => string; icon: string; run: (password: string) => Promise<void> } | null>(null);
  let error: Said | null = $state(null);
  /** What the last change did. */
  let note = $state<'security.app.off.done' | 'security.passkey.removed' | null>(null);
  const passkeysHere = passkeysInBrowser();
  const on = $derived(status !== null && (status.app || status.passkeys.length > 0));

  const page = requests(() => onLogin());
  async function load() {
    try {
      status = await api<SecondStep>('GET', '/api/security');
    } catch (e) {
      page.fail(e, (text) => (error = () => text));
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
  function explain(e: unknown): Said | null {
    if (statusOf(e) === 401) {
      onLogin();
      return null;
    }
    if (bodyOf(e).reason === 'password') return () => t(NOT_YOUR_PASSWORD);
    if (statusOf(e) === 429) return () => t(TOO_MANY_ATTEMPTS);
    const refused = passkeyRefusal(e);
    if (refused === 'untrusted') return () => t(UNTRUSTED_PASSKEY);
    if (refused === 'cancelled') return () => t('security.passkey.cancelled');
    if (refused === 'added') return () => t('security.passkey.held');
    const message = messageOf(e);
    return () => message;
  }

  function ask(title: () => string, action: () => string, icon: string, run: (password: string) => Promise<void>) {
    note = null;
    asking = { title, action, icon, run };
  }

  async function confirmed(password: string): Promise<Said | null> {
    try {
      await asking?.run(password);
      return null;
    } catch (e) {
      return explain(e);
    }
  }

  // Removing one factor with nothing else on says so: the password alone signs in then, and the codes go.
  const turnOffApp = () => {
    const last = (status?.passkeys.length ?? 0) === 0;
    ask(
      () => (last ? t('security.app.off.askLast') : t('security.app.off.ask')),
      () => t('security.app.turnOff'),
      ICONS.turnOff,
      async (password) => {
        await api('POST', '/api/security/app/remove', { password });
        changed(null);
        note = 'security.app.off.done';
      },
    );
  };
  const newPasskey = () =>
    ask(
      () => t('security.passkey.add.ask'),
      () => t('security.passkey.add.continue'),
      ICONS.passkey,
      async (password) => changed(await addPasskey(password)),
    );
  const removePasskey = (passkey: PasskeyInfo) => {
    const last = (status?.passkeys.length ?? 1) - 1 + (status?.app ? 1 : 0) === 0;
    ask(
      () => {
        const when = added(passkey.created);
        return last ? t('security.passkey.remove.askLast', { when }) : t('security.passkey.remove.ask', { when });
      },
      () => t('security.passkey.remove'),
      ICONS.delete,
      async (password) => {
        await api('POST', `/api/security/passkeys/${passkey.id}/remove`, { password });
        changed(null);
        note = 'security.passkey.removed';
      },
    );
  };
  const newCodes = () =>
    ask(
      () => t('security.codes.new.ask'),
      () => t('security.codes.new'),
      ICONS.retry,
      async (password) => changed((await api<{ recovery_codes: string[] }>('POST', '/api/security/recovery', { password })).recovery_codes),
    );
  /** When a passkey was added, to the second, which tells two apart: adding one takes longer than that. */
  const added = (seconds: number) =>
    new Date(seconds * 1000).toLocaleString(i18n.locale, { dateStyle: 'medium', timeStyle: 'medium' });
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
    <div class="max-w-xl"><AppSetup replacing={showing === 'replace'} onDone={changed} {onLogin} /></div>
  {:else if showing === 'codes'}
    <div class="max-w-xl"><RecoveryCodes {codes} onDone={() => (showing = 'overview')} /></div>
  {:else}
    <PageHeader icon={ICONS.security} title={t('security.title')} subtitle={t('security.subtitle')} />
    {#if status === null}
      {#if !error}<Skeleton said={t('security.loading')} rows={2} />{/if}
    {:else}
      {#if !on}
        <p class="mt-8 flex items-center gap-2.5 rounded-2xl bg-you/[0.07] px-4 py-3 text-sm font-semibold">
          <Icon d={ICONS.warning} class="size-4 shrink-0 text-[#b5452b]" />{t('security.alone')}
        </p>
      {/if}

      <div class="mt-8 -mx-3 space-y-2">
        {#snippet appActions()}
          {#if status?.app}
            <Button size="sm" icon={ICONS.replace} onclick={() => ((showing = 'replace'), (asking = null))}>{t('security.app.replace')}</Button>
            <Button variant="danger" size="sm" icon={ICONS.turnOff} onclick={turnOffApp}>{t('security.app.turnOff')}</Button>
          {:else}
            <Button variant="primary" size="sm" icon={ICONS.check} onclick={() => ((showing = 'app'), (asking = null))}>{t('security.app.turnOn')}</Button>
          {/if}
        {/snippet}
        {@render part(ICONS.app, t('security.app'), { on: status.app, text: status.app ? t('security.on') : t('security.off') }, t('security.app.line'), appActions)}

        {#snippet passkeyActions()}
          {#if status?.passkeys_possible && passkeysHere}
            <Button size="sm" icon={ICONS.add} onclick={newPasskey}>{t('security.passkey.add')}</Button>
          {/if}
        {/snippet}
        {@render part(
          ICONS.passkey,
          t('security.passkeys'),
          {
            on: status.passkeys.length > 0,
            text: status.passkeys.length === 0 ? t('security.passkeys.none') : t('security.passkeys.added', { count: status.passkeys.length }),
          },
          !status.passkeys_possible ? t('security.passkeys.ip') : !passkeysHere ? t('security.passkeys.browser') : t('security.passkeys.line'),
          passkeyActions,
        )}
        {#if status.passkeys.length > 0}
          <ul class="ml-[4.25rem] space-y-1">
            {#each status.passkeys as passkey (passkey.id)}
              <li class="flex items-center gap-3 py-1.5 {row}">
                <span class="min-w-0 flex-1 text-sm">{t('security.passkey')}<span class="text-muted"> · {t('security.passkey.added', { when: added(passkey.created) })}</span></span>
                <Button variant="danger" size="sm" icon={ICONS.delete} onclick={() => removePasskey(passkey)}>{t('security.passkey.remove')}</Button>
              </li>
            {/each}
          </ul>
        {/if}

        {#if on}
          {#snippet codeActions()}
            <Button size="sm" icon={ICONS.retry} onclick={newCodes}>{t('security.codes.new')}</Button>
          {/snippet}
          {@render part(
            ICONS.recovery,
            t('security.codes'),
            { on: status.recovery_codes > 0, text: t('security.codes.left', { count: status.recovery_codes }) },
            t('security.codes.line'),
            codeActions,
          )}
        {/if}
      </div>
      <p class="mt-4 text-[13px] text-muted">{t('security.signsOut')}</p>

      {#if asking}
        {#key asking}
          <PasswordCheck title={asking.title()} action={asking.action()} icon={asking.icon} onConfirm={confirmed} onCancel={() => (asking = null)} />
        {/key}
      {/if}
      <p class="mt-4 text-sm font-semibold text-good" role="status">{note ? t(note) : ''}</p>
    {/if}
    <p class="mt-3 font-semibold text-bad" role="alert">{error?.()}</p>
  {/if}
</div>
