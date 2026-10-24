<script lang="ts">
  // How the connection is doing, in one word and one number; a click opens the few numbers behind it.
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
      ? { word: 'Measuring', bars: 0, tone: 'text-muted' }
      : ms < 60
        ? { word: 'Excellent', bars: 4, tone: 'text-good' }
        : ms < 120
          ? { word: 'Good', bars: 3, tone: 'text-good' }
          : ms < 250
            ? { word: 'Fair', bars: 2, tone: 'text-fair' }
            : { word: 'Poor', bars: 1, tone: 'text-bad' },
  );
  const rows = $derived.by((): [string, string][] => {
    const s = stats;
    const traffic = s ? (s.bytes * 8) / 1000 : null;
    return [
      ['Latency (round trip)', ms === null ? '—' : `${ms} ms`],
      ['Last video delay', lastDelay === null ? '—' : `${Math.round(lastDelay / 1000)} ms`],
      ['Frames', s ? `${s.presented} fps` : '—'],
      ['Traffic', traffic === null ? '—' : traffic >= 1000 ? `${(traffic / 1000).toFixed(1)} Mbit/s` : `${traffic.toFixed(0)} kbit/s`],
      ['Video', s?.codec == null ? '—' : `${CODEC_NAMES[s.codec]}${s.hardware ? ' · hardware' : ''}`],
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
    <span>{grade.word}</span>{#if ms !== null}<span class="font-medium tabular-nums text-muted">{ms} ms</span>{/if}
  </button>
  {#if open}
    <div id="connection-details" class="absolute right-0 top-full z-20 mt-2 w-72 rounded-2xl border border-line bg-paper p-4 shadow-[0_24px_60px_-24px_rgb(20_40_75/0.45)]">
      <p class="text-sm font-semibold">Connection</p>
      <dl class="mt-2 grid grid-cols-[1fr_auto] gap-x-4 gap-y-1.5 text-sm">
        {#each rows as [k, v] (k)}<dt class="text-muted">{k}</dt><dd class="m-0 text-right font-semibold tabular-nums">{v}</dd>{/each}
      </dl>
      <p class="mt-3 text-xs text-muted">0 fps is normal while the page does not change.</p>
    </div>
  {/if}
</div>
