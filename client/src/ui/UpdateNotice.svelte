<script lang="ts">
  // Above every page of an installation's admin: a newer release than the one it runs is out, and the command that
  // moves it there. The release it runs is the admin's (`/api/me`); the latest, what https://easi.run/release.json names,
  // with the releases before it.
  // Hidden for a release, it stays hidden until a newer one is out.
  import Button from './Button.svelte';
  import { ICONS } from './icons.ts';
  import Icon from './Icon.svelte';
  import { t } from '../i18n/index.svelte.ts';
  import { copyText, me } from './api.ts';

  const COMMAND = 'curl -fsSLO https://easi.run/install.sh && bash install.sh --update';
  const HIDDEN = 'easi-run-update-hidden';
  const COMMIT = /^[0-9a-f]{40}$/;

  let latest: string | null = $state(null);
  let copied = $state(false);

  const hidden = (): string | null => {
    try {
      return localStorage.getItem(HIDDEN);
    } catch {
      return null;
    }
  };

  async function check() {
    const running = (await me()).release;
    if (!running) return;
    const answer = await fetch('https://easi.run/release.json', { cache: 'no-store', credentials: 'omit', referrerPolicy: 'no-referrer' });
    if (!answer.ok) return;
    const { commit, earlier } = (await answer.json()) as { commit?: unknown; earlier?: unknown };
    // Newer only when the release it runs is one of the earlier ones (a later one, or one never published, is not).
    const older = Array.isArray(earlier) && earlier.includes(running);
    if (typeof commit === 'string' && COMMIT.test(commit) && older && commit !== hidden()) latest = commit;
  }
  // Not reached (offline, say): nothing to say.
  check().catch(() => undefined);

  function hide() {
    try {
      if (latest) localStorage.setItem(HIDDEN, latest);
    } catch {
      // Hidden for this page only.
    }
    latest = null;
  }
</script>

{#if latest}
  <section class="mb-8 rounded-2xl border border-line bg-wash p-4 sm:p-5" aria-labelledby="update-title">
    <h2 id="update-title" class="flex items-center gap-2 text-[15px] font-semibold"><Icon d={ICONS.retry} class="size-5 shrink-0 text-accent" />{t('shell.update.title')}</h2>
    <p class="mt-1 text-sm text-muted">{t('shell.update.text')}</p>
    <code class="mt-3 block overflow-x-auto whitespace-nowrap rounded-lg bg-paper px-3 py-2 text-[13px]">{COMMAND}</code>
    <div class="mt-3 flex flex-wrap gap-2">
      <Button size="sm" icon={copied ? ICONS.check : ICONS.copy} onclick={async () => (copied = await copyText(COMMAND))}>{copied ? t('shell.update.copied') : t('shell.update.copy')}</Button>
      <Button size="sm" variant="ghost" icon={ICONS.close} onclick={hide}>{t('shell.update.hide')}</Button>
    </div>
  </section>
{/if}
