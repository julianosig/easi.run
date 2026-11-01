<script lang="ts">
  // A session's replay (see replay.ts): its screen as the agent left it moment by moment, the timeline's events beside
  // it (each one a place to jump to), and the player's controls under it: play, the previous and next event, the
  // position (with the events marked and what was not recorded shaded), the speed and skipping idle time. Over the
  // whole window; Escape closes it, Space plays or pauses, the arrows go from event to event.
  import Button from './Button.svelte';
  import Icon from './Icon.svelte';
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { api, statusOf, type TimelineEvent, type TimelinePage } from './api.ts';
  import { errorOf } from './errors.ts';
  import { focusRing } from './ui.ts';
  import { lines } from './timeline.ts';
  import { clock, parseRecords, Playback, type ReplayIndex, type ReplayRecord, unrecorded } from './replay.ts';
  import { ReplayScreen } from '../player.ts';
  import { probeCodecs } from '../decoder.ts';
  import { CODEC_AV1 } from '../protocol.ts';
  import { onMount, untrack } from 'svelte';

  let {
    id,
    label,
    live,
    at = null,
    onClose,
    onLogin,
  }: {
    id: string;
    /** What the session is called (its saved session, or its page). */
    label: string;
    /** A running session's: it keeps recording, and Back to live closes the replay. */
    live: boolean;
    /** Where to start: an event's time (Unix milliseconds), or the end of what is recorded. */
    at?: number | null;
    onClose: () => void;
    onLogin: () => void;
  } = $props();

  const SPEEDS = [1, 2, 4, 8] as const;
  let stage: HTMLDivElement | undefined = $state();
  // Replaced as a whole when a running session's recording grows (the playback holds the same object).
  let index = $state.raw<ReplayIndex | null>(null);
  let events: TimelineEvent[] = $state([]);
  let failure: Said | null = $state(null);
  let unsupported = $state(false);
  let time = $state(0);
  let end = $state(0);
  let playing = $state(false);
  let speed: (typeof SPEEDS)[number] = $state(1);
  let skipIdle = $state(true);
  // Made once the index is in; the screen draws into `stage`.
  let playback: Playback | null = $state.raw(null);
  let screen: ReplayScreen | null = null;

  const say = (key: string, params?: Record<string, string | number>) => t(`timeline.${key}` as Parameters<typeof t>[0], params as never);
  const said = $derived(lines(events, say));
  /** The events on the replay's clock (milliseconds from its start), the latest at or before now marked as current. */
  const marks = $derived(index ? said.map((l) => ({ line: l, t: l.event.at - index!.started })) : []);
  const current = $derived.by(() => {
    let found = -1;
    marks.forEach((m, i) => {
      if (m.t <= time) found = i;
    });
    return found;
  });
  const gaps = $derived(index ? unrecorded(index) : []);

  // Segments load as they are needed; the last few stay for going back and forth, no more (a whole recording would
  // weigh what it weighs on disk).
  const KEPT_SEGMENTS = 4;
  const segments = new Map<number, Promise<ReplayRecord[]>>();
  function segment(n: number): Promise<ReplayRecord[]> {
    let loaded = segments.get(n);
    if (loaded) {
      segments.delete(n);
      segments.set(n, loaded);
    } else {
      loaded = fetch(`/api/sessions/${id}/replay/${n}`, { cache: 'no-store', credentials: 'same-origin' }).then(async (r) => {
        if (r.status === 401) onLogin();
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return parseRecords(new Uint8Array(await r.arrayBuffer()));
      });
      loaded.catch(() => segments.delete(n));
      segments.set(n, loaded);
      while (segments.size > KEPT_SEGMENTS) segments.delete(segments.keys().next().value!);
    }
    return loaded;
  }

  async function readIndex(): Promise<ReplayIndex | null> {
    try {
      return await api<ReplayIndex>('GET', `/api/sessions/${id}/replay`);
    } catch (e) {
      if (statusOf(e) === 401) onLogin();
      else failure = errorOf(e);
      return null;
    }
  }

  async function readEvents() {
    try {
      events = (await api<TimelinePage>('GET', `/api/sessions/${id}/timeline?after=0`)).events;
    } catch {
      // The replay plays without its events.
    }
  }

  let root: HTMLDialogElement | undefined = $state();
  /** A segment that did not load: the replay stopped where it was; Retry seeks there again. */
  let stalled = $state(false);
  onMount(() => {
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    // A modal dialog: the page under it is inert (no focus reaches the live session's keyboard) until it closes.
    root?.showModal();
    // The window takes the keyboard (Space, the arrows, Escape) without a control looking chosen.
    root?.focus();
    let disposed = false;
    let frame = 0;
    let last = performance.now();
    const tick = (now: number) => {
      if (disposed) return;
      playback?.advance(now - last);
      last = now;
      if (playback) {
        time = playback.time;
        playing = playback.playing;
      }
      frame = requestAnimationFrame(tick);
    };
    void (async () => {
      const [found, probe] = await Promise.all([readIndex(), probeCodecs().catch(() => null), readEvents()]);
      if (disposed || found === null || !stage) return;
      unsupported = probe !== null && (probe.codecs & CODEC_AV1) === 0;
      index = found;
      end = found.segments.at(-1)?.end ?? 0;
      if (found.segments.length === 0) return;
      screen = new ReplayScreen(stage, t('demo.scene.cursor.agent'), (error) => (failure = () => t('replay.failed', { error: error.message })));
      playback = new Playback(found, segment, screen, () => (stalled = true));
      playback.skipIdle = skipIdle;
      const start = at === null ? Math.max(0, end - 5000) : at - found.started - 500;
      await playback.seek(start);
      if (disposed) return;
      time = playback.time;
      frame = requestAnimationFrame(tick);
    })();
    // A running session's recording goes on: what it adds lengthens the replay.
    const timer = live
      ? setInterval(async () => {
          const fresh = await readIndex();
          if (fresh && index && !disposed) {
            index = fresh;
            if (playback) playback.index = fresh;
            end = fresh.segments.at(-1)?.end ?? end;
            void readEvents();
          }
        }, 5000)
      : undefined;
    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      clearInterval(timer);
      playback?.dispose();
      screen?.destroy();
      if (before?.isConnected) before.focus();
    };
  });

  function toggle() {
    if (!playback) return;
    if (playback.time >= end) void playback.seek(0);
    playback.playing = !playback.playing;
    playing = playback.playing;
  }
  // The event the last step went to, while the replay stands exactly where that step put it (any other seek, or
  // playing on, forgets it): events at the same moment are then stepped through one by one.
  let stepped: { index: number; landed: number } | null = null;
  function seekTo(t: number, from: { index: number } | null = null) {
    if (!playback) return;
    stalled = false;
    stepped = from === null ? null : { index: from.index, landed: Math.max(0, Math.min(t, end)) };
    void playback.seek(t).then(() => (time = playback!.time));
    time = Math.max(0, Math.min(t, end));
  }
  /** Where event navigation stands: the event a step just went to, or the last whose landing (half a second before
   * it, where a step puts the replay) has passed. */
  function standing(): number {
    if (stepped !== null && stepped.landed === time) return stepped.index;
    let found = -1;
    marks.forEach((m, i) => {
      if (Math.max(0, m.t - 500) <= time + 10) found = i;
    });
    return found;
  }
  /** The previous or the next event in the list, within what is recorded. */
  function step(direction: -1 | 1) {
    for (let i = standing() + direction; i >= 0 && i < marks.length; i += direction) {
      const t = marks[i]!.t;
      if (t < 0 || t > end) continue;
      return seekTo(Math.max(0, t - 500), { index: i });
    }
  }
  $effect(() => {
    const s = speed;
    untrack(() => playback && (playback.speed = s));
  });
  $effect(() => {
    const skip = skipIdle;
    untrack(() => playback && (playback.skipIdle = skip));
  });

  // The position bar: a click or a drag seeks, the arrows step by 5 seconds.
  let track: HTMLDivElement | undefined = $state();
  let dragging = $state(false);
  const fraction = (t: number) => (end > 0 ? Math.min(100, Math.max(0, (t / end) * 100)) : 0);
  function pointAt(e: PointerEvent): number {
    const box = track!.getBoundingClientRect();
    return Math.min(1, Math.max(0, (e.clientX - box.left) / box.width)) * end;
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      return onClose();
    }
    // A focused control takes its own keys (Space presses a button, the arrows move the position).
    const target = e.target as HTMLElement;
    if (target.closest('button, a, select, input, [role="slider"]')) return;
    if (e.key === ' ') {
      e.preventDefault();
      toggle();
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      step(-1);
    } else if (e.key === 'ArrowRight') {
      e.preventDefault();
      step(1);
    }
  }
  const TONES = { plain: 'text-ink', problem: 'text-bad', waiting: 'text-accent', done: 'text-good' } as const;
  const ACTOR_ICONS = { agent: ICONS.agents, people: ICONS.watch, page: ICONS.website } as const;
  const MARK_TONES = { agent: 'bg-ink/45', people: 'bg-accent', page: 'bg-ink/25' } as const;
  const startedAt = $derived(index ? new Date(index.started).toLocaleString(i18n.locale, { dateStyle: 'medium', timeStyle: 'short' }) : '');
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={root}
  class="fixed inset-0 m-0 flex h-dvh max-h-none w-screen max-w-none flex-col border-0 bg-paper p-0 text-ink outline-none backdrop:bg-ink/40"
  aria-labelledby="replay-title"
  tabindex="-1"
  onkeydown={onKey}
  oncancel={(e) => {
    e.preventDefault();
    onClose();
  }}
>
  <header class="flex h-16 shrink-0 items-center gap-3 border-b border-line px-4 sm:px-6">
    <span class="grid size-9 shrink-0 place-items-center rounded-xl bg-accent-soft text-accent"><Icon d={ICONS.replay} class="size-5" /></span>
    <div class="min-w-0 flex-1">
      <h2 id="replay-title" class="flex items-center gap-2 text-[15px] font-semibold">
        {t('replay.title')}
        {#if live && index?.recording}<span class="inline-flex items-center gap-1.5 rounded-full bg-you/10 px-2 py-0.5 text-[12px] font-semibold text-you"><span class="size-1.5 rounded-full bg-you"></span>{t('replay.recording')}</span>{/if}
      </h2>
      <p class="truncate text-[13px] text-muted">{label}{startedAt ? ` · ${startedAt}` : ''}</p>
    </div>
    {#if live}
      <Button size="sm" icon={ICONS.watch} onclick={onClose} aria-label={t('replay.back')}><span class="hidden sm:inline">{t('replay.back')}</span></Button>
    {/if}
    <button class="grid size-9 shrink-0 place-items-center rounded-full text-muted transition hover:bg-wash hover:text-ink {focusRing}" type="button" aria-label={t('replay.close')} title={t('replay.close')} onclick={onClose}
      ><Icon d={ICONS.close} class="size-5" /></button
    >
  </header>

  <div class="grid min-h-0 flex-1 grid-rows-[minmax(0,1fr)_auto] lg:grid-cols-[minmax(0,1fr)_22rem] lg:grid-rows-1">
    <section class="flex min-h-0 flex-col bg-wash" aria-label={t('replay.title')}>
      <div class="relative min-h-0 flex-1 p-3 sm:p-6">
        <div bind:this={stage} class="grid h-full w-full place-items-center [&>canvas]:rounded-xl [&>canvas]:bg-void [&>canvas]:shadow-[0_18px_50px_-24px_rgb(20_40_75/0.55)]"></div>
        {#if stalled}
          <div class="absolute inset-x-0 bottom-6 grid place-items-center px-6">
            <p class="flex items-center gap-3 rounded-2xl bg-paper px-4 py-3 text-sm shadow-sm" role="alert">
              <span class="font-semibold text-bad">{t('replay.stalled')}</span>
              <Button size="sm" icon={ICONS.retry} onclick={() => seekTo(time)}>{t('replay.retry')}</Button>
            </p>
          </div>
        {/if}
        {#if failure || unsupported || !index || index.segments.length === 0}
          <div class="pointer-events-none absolute inset-0 grid place-items-center p-6">
            <p class="max-w-md rounded-2xl bg-paper/95 px-5 py-4 text-center text-sm shadow-sm {failure ? 'font-semibold text-bad' : 'text-muted'}" role={failure ? 'alert' : 'status'}>
              {#if failure}{failure()}{:else if !index}{t('replay.loading')}{:else if index.segments.length === 0}{t('replay.none')}{:else}{t('replay.unsupported')}{/if}
            </p>
          </div>
        {/if}
      </div>

      <div class="shrink-0 border-t border-line bg-paper px-4 pb-4 pt-3 sm:px-6">
        {#if index?.stopped === 'full' || index?.stopped === 'no-room' || index?.stopped === 'unwritten'}
          <p class="mb-2 text-[13px] font-semibold text-fair">{t(`replay.stopped.${index.stopped}`)}</p>
        {/if}
        <!-- The position: what is recorded, what is not (shaded), every event marked, and where the replay is. -->
        <div
          bind:this={track}
          class="group relative h-7 cursor-pointer touch-none select-none rounded-md {focusRing}"
          role="slider"
          tabindex="0"
          aria-label={t('replay.position')}
          aria-valuemin={0}
          aria-valuemax={Math.round(end / 1000)}
          aria-valuenow={Math.round(time / 1000)}
          aria-valuetext={t('replay.position.value', { at: clock(time), total: clock(end) })}
          onpointerdown={(e) => {
            if (!playback || end === 0) return;
            dragging = true;
            track!.setPointerCapture(e.pointerId);
            time = pointAt(e);
          }}
          onpointermove={(e) => dragging && (time = pointAt(e))}
          onpointerup={(e) => {
            if (!dragging) return;
            dragging = false;
            seekTo(pointAt(e));
          }}
          onkeydown={(e) => {
            if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
              e.preventDefault();
              seekTo(time + (e.key === 'ArrowLeft' ? -5000 : 5000));
            } else if (e.key === 'Home' || e.key === 'End') {
              e.preventDefault();
              seekTo(e.key === 'Home' ? 0 : end);
            }
          }}
        >
          <div class="absolute inset-x-0 top-1/2 h-1.5 -translate-y-1/2 overflow-hidden rounded-full bg-line">
            <div class="absolute inset-y-0 left-0 rounded-full bg-accent" style:width="{fraction(time)}%"></div>
            <!-- Over the progress too: what was not recorded stays visible wherever the replay is. -->
            {#each gaps as [from, to] (from)}
              <div
                class="absolute inset-y-0 bg-[repeating-linear-gradient(135deg,rgb(255_255_255/0.75)_0_3px,rgb(20_40_75/0.22)_3px_6px)]"
                style:left="{fraction(from)}%"
                style:width="{fraction(to) - fraction(from)}%"
                title={t('replay.unrecorded')}
              ></div>
            {/each}
          </div>
          {#each marks as m (m.line.event.id)}
            {#if m.t >= 0 && m.t <= end}
              <span
                class="pointer-events-none absolute top-1/2 h-3 w-[3px] -translate-x-1/2 -translate-y-1/2 rounded-full {m.line.tone === 'problem' ? 'bg-bad' : MARK_TONES[m.line.actor]}"
                style:left="{fraction(m.t)}%"
              ></span>
            {/if}
          {/each}
          <span
            class="pointer-events-none absolute top-1/2 size-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-paper bg-accent shadow transition-transform group-hover:scale-110 {dragging ? 'scale-125' : ''}"
            style:left="{fraction(time)}%"
          ></span>
        </div>

        <div class="mt-2 flex flex-wrap items-center gap-x-3 gap-y-2">
          <div class="flex items-center gap-1">
            <Button variant="ghost" size="sm" icon={ICONS.previous} label={t('replay.previous')} disabled={!playback} onclick={() => step(-1)} />
            <button
              class="grid size-10 place-items-center rounded-full bg-accent text-white shadow-[0_8px_20px_-10px_rgb(36_87_245/0.9)] transition hover:bg-accent-strong disabled:opacity-40 {focusRing}"
              type="button"
              aria-label={playing ? t('replay.pause') : t('replay.play')}
              title={playing ? t('replay.pause') : t('replay.play')}
              disabled={!playback}
              onclick={toggle}
            >
              <Icon d={playing ? ICONS.pause : ICONS.play} class="size-5" />
            </button>
            <Button variant="ghost" size="sm" icon={ICONS.following} label={t('replay.next')} disabled={!playback} onclick={() => step(1)} />
          </div>
          <span class="text-[13px] tabular-nums text-muted"><span class="font-semibold text-ink">{clock(time)}</span> / {clock(end)}</span>
          <div class="ml-auto flex items-center gap-2">
            <div class="flex rounded-full bg-wash p-0.5" role="group" aria-label={t('replay.speed')}>
              {#each SPEEDS as s (s)}
                <button
                  class="rounded-full px-2.5 py-1 text-[12px] font-semibold tabular-nums transition {focusRing} {speed === s ? 'bg-paper text-ink shadow-sm' : 'text-muted hover:text-ink'}"
                  type="button"
                  aria-pressed={speed === s}
                  onclick={() => (speed = s)}>{s}×</button
                >
              {/each}
            </div>
            <button
              class="grid size-9 place-items-center rounded-full transition {focusRing} {skipIdle ? 'bg-accent-soft text-accent' : 'text-muted hover:bg-wash hover:text-ink'}"
              type="button"
              aria-pressed={skipIdle}
              aria-label={t('replay.skip')}
              title={t('replay.skip')}
              onclick={() => (skipIdle = !skipIdle)}
            >
              <Icon d={ICONS.skipIdle} class="size-[18px]" />
            </button>
          </div>
        </div>
      </div>
    </section>

    <aside class="flex max-h-[38vh] min-h-0 flex-col border-t border-line bg-paper lg:max-h-none lg:border-l lg:border-t-0" aria-labelledby="replay-events">
      <h3 id="replay-events" class="flex items-center justify-between px-5 pb-2 pt-4 text-sm font-semibold">
        {t('replay.events')}<span class="text-[12px] font-normal tabular-nums text-muted">{marks.length}</span>
      </h3>
      {#if marks.length === 0}
        <p class="px-5 text-sm text-muted">{t('replay.events.none')}</p>
      {:else}
        <ol class="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
          {#each marks as m, i (m.line.event.id)}
            <li>
              <button
                class="flex w-full items-start gap-3 rounded-xl px-3 py-2 text-left transition {focusRing} {i === current ? 'bg-accent-soft' : 'hover:bg-wash'}"
                type="button"
                aria-current={i === current ? 'true' : undefined}
                onclick={() => seekTo(Math.max(0, m.t - 500), { index: i })}
              >
                <Icon d={ACTOR_ICONS[m.line.actor]} class="mt-0.5 size-4 shrink-0 {i === current ? 'text-accent' : 'text-muted'}" />
                <span class="min-w-0 flex-1">
                  <span class="block text-[13px] leading-snug {TONES[m.line.tone]}">{m.line.text}{#if m.line.times > 1}<span class="ml-1 text-muted">{say('times', { count: m.line.times })}</span>{/if}</span>
                  {#if m.line.outcome}<span class="block text-[12px] text-bad">{m.line.outcome}</span>{/if}
                </span>
                <span class="shrink-0 pt-0.5 text-[12px] tabular-nums text-muted">{clock(Math.max(0, m.t))}</span>
              </button>
            </li>
          {/each}
        </ol>
      {/if}
    </aside>
  </div>
</dialog>
