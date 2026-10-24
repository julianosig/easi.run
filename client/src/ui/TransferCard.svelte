<script lang="ts">
  // A batch of files on its way to the page, drawn where they were dropped (or centred, for a file chooser): the bytes
  // the server has received so far, then a tick, then nothing, as the page's own interface takes over.
  import type { Transfer } from '../viewer.ts';
  import { formatSize } from './api.ts';

  let { transfer, width, height }: { transfer: Transfer; width: number; height: number } = $props();

  const CARD_W = 264;
  const CARD_H = 76;
  const percent = $derived(transfer.total === 0 ? 100 : Math.floor((100 * transfer.received) / transfer.total));
  const label = $derived(transfer.files === 1 ? transfer.name : `${transfer.files} files`);
  const status = $derived(
    transfer.state === 'done'
      ? 'Uploaded'
      : transfer.state === 'failed'
        ? 'Upload failed'
        : transfer.state === 'finishing'
          ? 'Finishing…'
          : `${percent}% · ${formatSize(transfer.received)} of ${formatSize(transfer.total)}`,
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
      <svg viewBox="0 0 24 24" class="size-4 shrink-0 text-good" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m5 12 5 5 9-10" /></svg>
    {:else}
      <svg viewBox="0 0 24 24" class="size-4 shrink-0 text-accent" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 19V7M7 12l5-5 5 5M5 4h14" /></svg>
    {/if}
    <span class="truncate">{label}</span>
  </p>
  <div
    class="mt-2 h-1.5 overflow-hidden rounded-full bg-line"
    role="progressbar"
    aria-label="Uploading {label}"
    aria-valuemin="0"
    aria-valuemax="100"
    aria-valuenow={percent}
  >
    <div class="h-full rounded-full transition-[width] duration-150 {transfer.state === 'failed' ? 'bg-bad' : transfer.state === 'done' ? 'bg-good' : 'bg-accent'}" style="width: {percent}%"></div>
  </div>
  <p class="mt-1.5 text-xs font-medium tabular-nums {transfer.state === 'failed' ? 'text-bad' : 'text-muted'}">
    {status}{#if transfer.left > 0} · {transfer.left} left out{/if}
  </p>
</div>
