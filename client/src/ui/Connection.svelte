<script lang="ts">
  // How the connection is doing, in one word and one number; a click opens the few numbers behind it.
  import { t } from '../i18n/index.svelte.ts';
  import type { ViewerStats } from '../viewer.ts';
  import { dismissable } from './dismiss.ts';
  import { CODEC_NAMES, focusRing } from './ui.ts';

  let { stats, open = $bindable(false) }: { stats: ViewerStats | null; open?: boolean } = $props();

  let root: HTMLDivElement | undefined = $state();
  // The last video delay measured: an unchanged page sends no video, and that is healthy. A dropped stream (no stats)
  // forgets it, so the next connection never shows the old one's delay as its own.
  let lastDelay: number | null = $state(null);
  $effect(() => {
    if (stats === null) lastDelay = null;
    else if (stats.composeToDrawUsP50 != null) lastDelay = stats.composeToDrawUsP50;
  });

  const ms = $derived(stats?.roundTripUs == null ? null : Math.round(stats.roundTripUs / 1000));
  const grade = $derived(
    ms === null
      ? { word: t('session.connection.measuring'), bars: 0, tone: 'text-muted' }
      : ms < 60
        ? { word: t('session.connection.excellent'), bars: 4, tone: 'text-good' }
        : ms < 120
          ? { word: t('session.connection.good'), bars: 3, tone: 'text-good' }
          : ms < 250
            ? { word: t('session.connection.fair'), bars: 2, tone: 'text-fair' }
            : { word: t('session.connection.poor'), bars: 1, tone: 'text-bad' },
  );
  /** The video's codec as the person reads it: lossless tiles when there is none, and whether it decodes in hardware. */
  function codecText(s: ViewerStats | null): string {
    if (s?.codec == null) return '—';
    const name = CODEC_NAMES[s.codec] ?? t('session.connection.lossless');
    return s.hardware ? t('session.connection.hardware', { codec: name }) : name;
  }
  const rows = $derived.by((): [string, string][] => {
    const s = stats;
    const traffic = s ? (s.bytes * 8) / 1000 : null;
    return [
      [t('session.connection.latency'), ms === null ? '—' : t('session.connection.ms', { ms })],
      [t('session.connection.delay'), lastDelay === null ? '—' : t('session.connection.ms', { ms: Math.round(lastDelay / 1000) })],
      [t('session.connection.frames'), s ? t('session.connection.fps', { fps: s.presented }) : '—'],
      [
        t('session.connection.traffic'),
        traffic === null
          ? '—'
          : traffic >= 1000
            ? t('session.connection.mbits', { rate: (traffic / 1000).toFixed(1) })
            : t('session.connection.kbits', { rate: traffic.toFixed(0) }),
      ],
      [t('session.connection.video'), codecText(s)],
    ];
  });

  $effect(() => {
    if (open) return dismissable(root, () => (open = false));
  });
</script>

<div class="relative" bind:this={root}>
  <button
    class="flex items-center gap-2 rounded-full px-3 py-2 text-sm font-semibold transition hover:bg-wash {focusRing} {open ? 'bg-wash' : ''}"
    type="button"
    aria-expanded={open}
    aria-controls="connection-details"
    onclick={() => (open = !open)}
  >
    <svg viewBox="0 0 20 16" class="h-4 w-5 {grade.tone}" aria-hidden="true">
      {#each [0, 1, 2, 3] as i (i)}
        <rect x={i * 5.2} y={12 - i * 3.6} width="3.6" height={4 + i * 3.6} rx="1" fill="currentColor" fill-opacity={i < grade.bars ? 1 : 0.18} />
      {/each}
    </svg>
    <span>{grade.word}</span>{#if ms !== null}<span class="font-medium tabular-nums text-muted">{t('session.connection.ms', { ms })}</span>{/if}
  </button>
  {#if open}
    <div id="connection-details" class="absolute right-0 top-full z-20 mt-2 w-72 rounded-2xl border border-line bg-paper p-4 shadow-[0_24px_60px_-24px_rgb(20_40_75/0.45)]">
      <p class="text-sm font-semibold">{t('session.connection')}</p>
      <dl class="mt-2 grid grid-cols-[1fr_auto] gap-x-4 gap-y-1.5 text-sm">
        {#each rows as [k, v] (k)}<dt class="text-muted">{k}</dt><dd class="m-0 text-right font-semibold tabular-nums">{v}</dd>{/each}
      </dl>
      <p class="mt-3 text-xs text-muted">{t('session.connection.idle')}</p>
    </div>
  {/if}
</div>
