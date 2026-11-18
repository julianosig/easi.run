<script lang="ts">
  // What became of the sessions that ran as easi.run restarted (`GET /api/runs`, its `resumes`): how many wait to start
  // again (until the password is back, when it is what they wait for), and the starts that failed. Nothing when none.
  import { followLists } from './updates.ts';
  import Button from './Button.svelte';
  import { ICONS } from './icons.ts';
  import { t } from '../i18n/index.svelte.ts';
  import { api, type Resumes, statusOf } from './api.ts';
  import { saidOf } from './errors.ts';
  import Unlock from './Unlock.svelte';

  let { onLogin }: { onLogin: () => void } = $props();

  /** How many archives only the password opens: the sessions that wait, wait for it too. */
  let locked = $state(0);
  let resumes: Resumes = $state({ waiting: 0, failed: [] });
  let unlocking = $state(false);

  // The latest load's answer only.
  let turn = 0;
  async function load() {
    const mine = ++turn;
    try {
      const listed = await api<{ locked: number; resumes?: Resumes }>('GET', '/api/runs');
      if (mine === turn) [locked, resumes] = [listed.locked, listed.resumes ?? { waiting: 0, failed: [] }];
    } catch (e) {
      if (mine === turn && statusOf(e) === 401) onLogin();
    }
  }
  $effect(() => {
    void load();
    return followLists(() => void load());
  });
</script>

{#if resumes.waiting > 0}
  <div class="mt-8 flex flex-wrap items-center justify-between gap-4 rounded-2xl bg-wash px-5 py-4">
    <p class="max-w-lg text-sm font-semibold text-ink">{t(locked > 0 ? 'timeline.runs.resume.locked' : 'timeline.runs.resume.starting', { count: resumes.waiting })}</p>
    {#if locked > 0}
      <Button variant="primary" icon={ICONS.unlock} onclick={() => (unlocking = true)}>{t('sessions.unlock')}</Button>
    {/if}
  </div>
{/if}
{#each resumes.failed as failed (failed.was)}
  <p class="mt-2 text-sm font-semibold text-bad" role="alert">{t(failed.chat ? 'timeline.runs.resume.chat' : 'timeline.runs.resume.failed', { name: failed.label || t('timeline.runs.resume.unnamed'), error: saidOf(failed)() })}</p>
{/each}

<Unlock bind:open={unlocking} subtitle={t('timeline.runs.resume.locked', { count: resumes.waiting })} onUnlocked={() => void load()} {onLogin} />
