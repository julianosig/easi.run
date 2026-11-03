<script lang="ts">
  // What one running session's containers use now, and how many viewers watch it.
  import Dialog from './Dialog.svelte';
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { formatSize } from './api.ts';
  import { errorOf } from './errors.ts';

  let { open = $bindable(false), told }: { open?: boolean; /** What the containers use, as told. */ told: string | null } = $props();

  /** What each container uses now, and how many viewers watch. */
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

  // Told while it is open (see `onSessionUsage` in Session).
  $effect(() => {
    if (!told) {
      usage = null;
      usageError = null;
      return;
    }
    try {
      const got = JSON.parse(told) as { containers: { containers?: Record<string, Container>; error?: string }; viewers: number };
      usage = { containers: got.containers.containers ?? {}, viewers: got.viewers };
      usageError = got.containers.error ? () => got.containers.error ?? '' : null;
    } catch (e) {
      usageError = errorOf(e);
    }
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
