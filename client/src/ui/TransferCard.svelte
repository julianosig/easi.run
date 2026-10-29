<script lang="ts">
  // A batch of files on its way to the page, drawn where they were dropped (or centred, for a file chooser): the bytes
  // the server has received so far, then a tick, then nothing, as the page's own interface takes over.
  import { i18n, t } from '../i18n/index.svelte.ts';
  import type { Transfer } from '../viewer.ts';
  import { formatSize } from './api.ts';
  import Icon from './Icon.svelte';

  let { transfer, width, height }: { transfer: Transfer; width: number; height: number } = $props();

  const CARD_W = 264;
  const CARD_H = 76;
  const percent = $derived(transfer.total === 0 ? 100 : Math.floor((100 * transfer.received) / transfer.total));
  const label = $derived(transfer.files === 1 ? transfer.name : t('session.card.files', { count: transfer.files }));
  const status = $derived(
    transfer.state === 'done'
      ? t('session.card.done')
      : transfer.state === 'failed'
        ? t('session.card.failed')
        : transfer.state === 'finishing'
          ? t('session.card.finishing')
          : t('session.card.progress', { percent, received: formatSize(transfer.received, i18n.locale), total: formatSize(transfer.total, i18n.locale) }),
  );
  // Next to where the files were dropped, inside the screen.
  const left = $derived(Math.max(8, Math.min(width - CARD_W - 8, (transfer.at?.x ?? width / 2) - CARD_W / 2)));
  const top = $derived(Math.max(8, Math.min(height - CARD_H - 8, (transfer.at?.y ?? height / 2) + 18)));
</script>

<div
  class="pointer-events-none absolute z-10 rounded-2xl bg-paper/95 px-4 py-3 shadow-[0_18px_44px_-14px_rgb(20_40_75/0.55)] ring-1 ring-line backdrop-blur"
  style="left: {left}px; top: {top}px; width: {CARD_W}px"
>
  <p class="flex items-center gap-2 text-sm font-semibold">
    {#if transfer.state === 'done'}
      <Icon d="m5 12 5 5 9-10" class="size-4 shrink-0 text-good" stroke={2.6} />
    {:else}
      <Icon d="M12 19V7M7 12l5-5 5 5M5 4h14" class="size-4 shrink-0 text-accent" stroke={2} />
    {/if}
    <span class="truncate">{label}</span>
  </p>
  <div
    class="mt-2 h-1.5 overflow-hidden rounded-full bg-line"
    role="progressbar"
    aria-label={t('session.card.uploading', { name: label })}
    aria-valuemin="0"
    aria-valuemax="100"
    aria-valuenow={percent}
  >
    <div class="h-full rounded-full transition-[width] duration-150 {transfer.state === 'failed' ? 'bg-bad' : transfer.state === 'done' ? 'bg-good' : 'bg-accent'}" style="width: {percent}%"></div>
  </div>
  <p class="mt-1.5 text-xs font-medium tabular-nums {transfer.state === 'failed' ? 'text-bad' : 'text-muted'}">
    {transfer.left > 0 ? t('session.card.leftOut', { status, count: transfer.left }) : status}
  </p>
</div>
