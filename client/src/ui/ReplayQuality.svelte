<script lang="ts" module>
  /** The saves, one after another across every time this page opens in a tab: a page left with a save queued never
   * overwrites the next page's. */
  let saves = Promise.resolve();
</script>

<script lang="ts">
  // How sharp the replays recorded from now on are, and about what that costs on disk; a change applies at once. Part of
  // the Settings page, as the API's `/api/replay-quality`.
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { row, rowBadge, rows } from './admin.ts';
  import { api, formatSize, type ReplayQuality } from './api.ts';
  import { requests } from './requests.svelte.ts';
  import Icon from './Icon.svelte';
  import { focusRing } from './ui.ts';
  import Skeleton from './Skeleton.svelte';

  let { onLogin }: { onLogin: () => void } = $props();
  const page = requests(() => onLogin());

  const SIZES = [100, 75, 50] as const;
  const SHARPNESS = ['high', 'medium', 'low'] as const;
  /** About the megabytes an hour of browsing real sites without a pause takes: 75% and low measured whole (19 MB), the
   * others from the display's encoder on the same recording, in proportion to it. */
  const MEGABYTES_AN_HOUR: Record<(typeof SIZES)[number], Record<(typeof SHARPNESS)[number], number>> = {
    100: { high: 110, medium: 65, low: 33 },
    75: { high: 69, medium: 39, low: 19 },
    50: { high: 39, medium: 22, low: 10 },
  };

  /** The choice as the page shows it, and as the admin last confirmed it. */
  let quality: ReplayQuality | null = $state.raw(null);
  let confirmed: ReplayQuality | null = null;
  let error: Said | null = $state(null);
  // Read in the same queue as the saves, so a save an earlier page left under way lands first.
  $effect(() => {
    saves = saves.then(async () => {
      try {
        const got = await api<ReplayQuality>('GET', '/api/replay-quality');
        if (page.alive()) quality = confirmed = got;
      } catch (e) {
        page.fail(e, (said) => (error = said));
      }
    });
  });
  /** Each save sends the page's choice as it is then, and one a later change overtook is skipped, so the last change is
   * what the admin keeps. A refused save puts back what the admin has. */
  function set(change: Partial<ReplayQuality>) {
    if (!quality) return;
    const wanted = (quality = { ...quality, ...change });
    error = null;
    saves = saves.then(async () => {
      if (!page.alive() || quality !== wanted) return;
      try {
        const { scale, quality: sharpness, disk_percent } = wanted;
        const saved = await api<ReplayQuality>('PUT', '/api/replay-quality', { scale, quality: sharpness, disk_percent });
        confirmed = saved;
        if (page.alive() && quality === wanted) quality = saved;
      } catch (e) {
        // A later change, still to be saved, says how it went instead.
        if (quality !== wanted) return;
        quality = confirmed;
        page.fail(e, (said) => (error = said));
      }
    });
  }
  const choice = `choice cursor-pointer rounded-lg bg-wash py-2 pl-3 pr-8 text-sm font-semibold text-ink transition hover:bg-accent-soft ${focusRing}`;
</script>

<section class="mt-12" aria-labelledby="settings-replay">
  <h2 id="settings-replay" class="text-lg font-extrabold tracking-[-0.02em]">{t('agents.replay.title')}</h2>
  <p class="mt-0.5 text-sm text-muted">{t('agents.replay.subtitle')}</p>
  <p class="mt-3 font-semibold text-bad empty:hidden" role="alert">{error?.()}</p>

  {#if quality}
    <ul class="mt-3 {rows}">
      <li class="flex flex-wrap items-center gap-4 py-3 sm:flex-nowrap {row}">
        <span class={rowBadge}><Icon d={ICONS.sessions} class="size-5" /></span>
        <span class="min-w-0 flex-1">
          <label class="block font-semibold" for="replay-size">{t('agents.replay.size')}</label>
          <span class="mt-1 block text-sm text-muted">{t('agents.replay.size.help')}</span>
        </span>
        <span class="relative shrink-0">
          <select id="replay-size" class={choice} value={quality.scale} onchange={(e) => set({ scale: Number(e.currentTarget.value) as ReplayQuality['scale'] })}>
            {#each SIZES as size (size)}<option value={size}>{size}%</option>{/each}
          </select>
          <Icon d={ICONS.collapse} class="pointer-events-none absolute right-2.5 top-1/2 size-4 -translate-y-1/2 text-muted" />
        </span>
      </li>
      <li class="flex flex-wrap items-center gap-4 py-3 sm:flex-nowrap {row}">
        <span class={rowBadge}><Icon d={ICONS.watch} class="size-5" /></span>
        <span class="min-w-0 flex-1">
          <label class="block font-semibold" for="replay-quality">{t('agents.replay.quality')}</label>
          <span class="mt-1 block text-sm text-muted">{t('agents.replay.quality.help')}</span>
        </span>
        <span class="relative shrink-0">
          <select id="replay-quality" class={choice} value={quality.quality} onchange={(e) => set({ quality: e.currentTarget.value as ReplayQuality['quality'] })}>
            {#each SHARPNESS as sharpness (sharpness)}<option value={sharpness}>{t(`agents.replay.quality.${sharpness}`)}</option>{/each}
          </select>
          <Icon d={ICONS.collapse} class="pointer-events-none absolute right-2.5 top-1/2 size-4 -translate-y-1/2 text-muted" />
        </span>
      </li>
      <li class="flex flex-wrap items-center gap-4 py-3 sm:flex-nowrap {row}">
        <span class={rowBadge}><Icon d={ICONS.download} class="size-5" /></span>
        <span class="min-w-0 flex-1">
          <label class="block font-semibold" for="replay-disk">{t('agents.replay.disk')}</label>
          <span class="mt-1 block text-sm text-muted">{t('agents.replay.disk.help')}</span>
        </span>
        <span class="relative flex shrink-0 items-center gap-1.5">
          <input
            id="replay-disk"
            class="w-20 rounded-lg bg-wash px-3 py-2 text-right text-sm font-semibold text-ink {focusRing}"
            type="number"
            min="1"
            max="90"
            step="1"
            value={quality.disk_percent}
            onchange={(e) => {
              const percent = Math.round(e.currentTarget.valueAsNumber);
              if (percent >= 1 && percent <= 90) set({ disk_percent: percent });
              else e.currentTarget.value = String(quality?.disk_percent ?? 10);
            }}
          />
          <span class="text-sm font-semibold text-muted">%</span>
        </span>
      </li>
    </ul>
    {#if quality.disk !== undefined && quality.used !== undefined}
      <p class="mt-3 text-sm text-muted">
        {t('agents.replay.disk.use', { limit: formatSize((quality.disk / 100) * quality.disk_percent, i18n.locale), used: formatSize(quality.used, i18n.locale) })}
      </p>
    {/if}
    <p class="mt-1 text-sm text-muted">{t('agents.replay.forecast', { size: formatSize(MEGABYTES_AN_HOUR[quality.scale][quality.quality] * 1_000_000, i18n.locale) })}</p>
  {:else if !error}
    <Skeleton said={t('agents.replay.loading')} />
  {/if}
</section>
