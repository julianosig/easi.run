<script lang="ts">
  // What one running session's containers use now, and how many viewers watch it.
  import Dialog from './Dialog.svelte';
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { api, formatSize, statusOf } from './api.ts';
  import { errorOf } from './errors.ts';
  import { untrack } from 'svelte';

  let { id, open = $bindable(false), onLogin }: { id: string; open?: boolean; onLogin: () => void } = $props();

  /** What each container uses now (`GET /api/sessions/{id}/stats`), and how many viewers watch. */
  interface Container {
    status?: string;
    running?: boolean;
    cpu_percent?: number | null;
    memory_bytes?: number | null;
    memory_limit_bytes?: number | null;
    error?: string;
  }
  let usage: { containers: Record<string, Container>; viewers: number } | null = $state(null);
  let usageError: Said | null = $state(null);

  /** The latest read: an earlier one's late answer (from before a reopening) changes nothing. */
  let turn = 0;
  // A read under way: the next tick waits for it, so a slow answer is never overtaken and dropped.
  let pending = false;
  async function read() {
    const mine = ++turn;
    pending = true;
    usageError = null;
    try {
      const got = await api<{ containers: { containers?: Record<string, Container>; error?: string }; viewers: unknown[] }>('GET', `/api/sessions/${id}/stats`);
      if (mine !== turn || !open) return;
      usage = { containers: got.containers.containers ?? {}, viewers: got.viewers.length };
      if (got.containers.error) usageError = () => got.containers.error ?? '';
    } catch (e) {
      if (mine !== turn || !open) return;
      if (statusOf(e) === 401) return onLogin();
      usageError = errorOf(e);
    } finally {
      if (mine === turn) pending = false;
    }
  }

  // Read as it opens and every few seconds while it stays open: each read answers at once, and the CPU a container
  // used comes from the read before (none on the first).
  const EVERY_MS = 3000;
  $effect(() => {
    if (!open) return;
    untrack(() => {
      usage = null;
      void read();
    });
    const timer = setInterval(() => pending || read(), EVERY_MS);
    return () => clearInterval(timer);
  });

  const number = (v: number, digits = 0) => v.toLocaleString(i18n.locale, { maximumFractionDigits: digits });
</script>

<Dialog title={t('session.usage.title')} icon={ICONS.usage} bind:open>
  <p class="mt-2 text-sm font-semibold text-bad empty:hidden" role="alert">{usageError?.()}</p>
  {#if usage}
    <p class="mt-2 text-sm text-muted">{t('session.usage.viewers', { count: usage.viewers })}</p>
    <dl class="mt-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-sm">
      {#each Object.entries(usage.containers) as [role, c] (role)}
        <dt class="font-semibold">{role}</dt>
        <dd class="m-0 text-right tabular-nums text-muted">
          {#if c.error}{c.error}
          {:else if !c.running}{t('session.usage.stopped', { status: c.status ?? '' })}
          {:else}{t('session.usage.cpu', { percent: c.cpu_percent == null ? '—' : number(c.cpu_percent, 1) })} · {c.memory_bytes == null
              ? '—'
              : c.memory_limit_bytes
                ? t('session.usage.memory', { used: formatSize(c.memory_bytes, i18n.locale), limit: formatSize(c.memory_limit_bytes, i18n.locale) })
                : formatSize(c.memory_bytes, i18n.locale)}{/if}
        </dd>
      {/each}
    </dl>
  {/if}
</Dialog>
