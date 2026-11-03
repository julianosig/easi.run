<script lang="ts">
  // A session's timeline and replay (see timeline.ts, replay.ts): its screen as the agent left it moment by moment, the
  // timeline's events beside it (each one a place to jump to, filtered by who it was about or what went wrong, what is
  // new since this browser last showed them summed up, and the whole downloadable as Markdown or JSON), and the
  // player's controls under it: play, the previous and next event, the position (with the events shown marked and what
  // was not recorded shaded), the speed and skipping idle time. Over the whole window; Escape closes it, Space plays or
  // pauses, the arrows go from event to event. A session with nothing recorded shows its events alone.
  import { throttled } from './throttle.ts';
  import Button from './Button.svelte';
  import Icon from './Icon.svelte';
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { api, statusOf, type TimelineEvent, type TimelinePage } from './api.ts';
  import { errorOf } from './errors.ts';
  import { focusRing } from './ui.ts';
  import { type Filter, leftAt, type Line, lines, markdown, markSeen, seenOf, shown, summary } from './timeline.ts';
  import { clock, parseRecords, Playback, type ReplayIndex, type ReplayRecord, unrecorded } from './replay.ts';
  import { ReplayScreen } from '../player.ts';
  import { probeCodecs } from '../decoder.ts';
  import { CODEC_AV1 } from '../protocol.ts';
  import { onMount, tick as rendered, untrack } from 'svelte';

  let {
    id,
    label,
    live,
    revision = 0,
    at = null,
    onClose,
    onLogin,
  }: {
    id: string;
    /** What the session is called (its saved session, or its page). */
    label: string;
    /** A running session's: it keeps recording, and Back to live closes the replay. */
    live: boolean;
    /** Moves on as what it holds does, while it is live (see `onSessionState` in Session). */
    revision?: number;
    /** Where to start: an event's time (Unix milliseconds), or the end of what is recorded. */
    at?: number | null;
    onClose: () => void;
    onLogin: () => void;
  } = $props();

  const SPEEDS = [0.5, 1, 2, 4, 8] as const;
  let stage: HTMLDivElement | undefined = $state();
  // Replaced as a whole when a running session's recording grows (the playback holds the same object).
  let index = $state.raw<ReplayIndex | null>(null);
  let events: TimelineEvent[] = $state([]);
  /** Why the screen cannot be played: the recording's index did not load, or a segment did not draw. */
  let failure: Said | null = $state(null);
  /** Why the events did not load (Retry reads them again), apart from the screen: either shows without the other. */
  let eventsFailure: Said | null = $state(null);
  /** Whether the session records no replay at all (the admin answers none: it was started with recording off). */
  let off = $state(false);
  /** Whether the first answers are in, the events' and the recording's: until then they say nothing rather than none. */
  let loaded = $state(false);
  let indexed = $state(false);
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
  let filter: Filter = $state('all');
  /** Whether there is a recording to play: until there is, the events show alone, with no time on its clock. */
  const recorded = $derived(index !== null && index.segments.length > 0);
  /** The events shown (the filter's), each with its time on the replay's clock (milliseconds from its start) once
   * there is a recording, none before. */
  const rows = $derived(said.filter((l) => shown(l, filter)).map((l) => ({ line: l, t: recorded ? l.event.at - index!.started : null })));
  /** What the position bar marks and the steps go through: the rows on the replay's clock. */
  const marks = $derived(rows.filter((r): r is { line: Line; t: number } => r.t !== null));
  /** The row whose details are open below it (what the agent wrote, who made it, a batch's steps): the one last
   * clicked or tapped, so they need no hover. */
  let opened: number | null = $state(null);
  // What was new when it opened, summed up while it stays open; the oldest event the admin still keeps; what the admin
  // calls the session, scrubbed as its events are (the export's title).
  const seen = untrack(() => seenOf(id));
  const fresh = $derived(events.filter((e) => e.seq > seen));
  const news = $derived(summary(fresh));
  const left = $derived(leftAt(events));
  let first = $state(1);
  let named = $state('');
  /** Where going to an event puts the replay: half a second before it, to see it happen. */
  const landing = (t: number) => Math.max(0, t - 500);
  /** The event marked as current: the one gone to, until the replay reaches a later one, or else the last whose
   * landing has passed (events at the same moment as the one gone to stay behind it). */
  const current = $derived.by(() => {
    let found = -1;
    if (!playback) return found;
    const gone = stepped !== null && time >= stepped.landed ? marks.findIndex((m) => m.line.event.id === stepped!.id) : -1;
    marks.forEach((m, i) => {
      const at = landing(m.t);
      if (gone < 0 ? at <= time + 10 : i === gone || (i > gone && at > stepped!.landed && at <= time)) found = Math.max(found, i);
    });
    return found;
  });
  const gaps = $derived(index ? unrecorded(index) : []);
  const currentId = $derived(current >= 0 ? marks[current]!.line.event.id : null);
  /** Whether the screen and its controls show: while the recording is looked for, once there is one, or to say why it
   * did not load; otherwise the events take the whole window. */
  const player = $derived(!indexed || recorded || failure !== null);

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

  /** The recording's index; none when it did not load (said in `failure`) or the session records none (`off`). */
  async function readIndex(): Promise<ReplayIndex | null> {
    try {
      const found = await api<ReplayIndex>('GET', `/api/sessions/${id}/replay`);
      if (playback === null) failure = null;
      return found;
    } catch (e) {
      if (statusOf(e) === 401) onLogin();
      else if (statusOf(e) === 404) off = true;
      else failure = errorOf(e);
      return null;
    }
  }

  // One read of the events at a time: a poll while one is out waits for the next.
  let reading = false;
  let readAgain = false;
  let closed = false;
  async function readEvents() {
    // One at a time, and a change during it reads again after it.
    if (reading) {
      readAgain = true;
      return;
    }
    reading = true;
    try {
      const page = await api<TimelinePage>('GET', `/api/sessions/${id}/timeline?after=0`);
      [events, first, eventsFailure] = [page.events, page.first, null];
      if (typeof page.label === 'string') named = page.label;
      markSeen(id, page.next - 1);
    } catch (e) {
      if (statusOf(e) === 401) onLogin();
      else eventsFailure = errorOf(e);
    } finally {
      reading = false;
      loaded = true;
      if (readAgain && !closed) {
        readAgain = false;
        void readEvents();
      }
    }
  }

  /** Takes `found` as the recording's index, and starts playing it the first time it holds something. */
  async function begin(found: ReplayIndex, disposed: () => boolean) {
    index = found;
    end = found.segments.at(-1)?.end ?? end;
    if (playback) {
      playback.index = found;
      return;
    }
    if (found.segments.length === 0) return;
    // The screen's place is drawn once there is something to play.
    await rendered();
    if (disposed() || playback || !stage) return;
    screen = new ReplayScreen(stage, t('demo.scene.cursor.agent'), (error) => (failure = () => t('replay.failed', { error: error.message })));
    playback = new Playback(found, segment, screen, () => (stalled = true));
    playback.skipIdle = skipIdle;
    await playback.seek(at === null ? Math.max(0, end - 5000) : at - found.started - 500);
    if (!disposed()) time = playback.time;
  }

  /** What a live session's changes read again, at most this often while an agent works. */
  const GROWS_EVERY_MS = 3000;
  let grown: ReturnType<typeof throttled> | undefined;
  let seenRevision = untrack(() => revision);
  $effect(() => {
    if (live && revision !== seenRevision) {
      seenRevision = revision;
      grown?.ask();
    }
  });

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
    frame = requestAnimationFrame(tick);
    void (async () => {
      const [found, probe] = await Promise.all([readIndex(), probeCodecs().catch(() => null), readEvents()]);
      indexed = true;
      unsupported = probe !== null && (probe.codecs & CODEC_AV1) === 0;
      if (found && !disposed) await begin(found, () => disposed);
    })();
    // A running session goes on: what it records lengthens the replay (or starts it), and its events come in.
    // One read of the index at a time, and a change during it reads again after it: an older answer never replaces a newer.
    let indexing = false;
    let indexAgain = false;
    const readGrown = async (): Promise<void> => {
      if (indexing) {
        indexAgain = true;
        return;
      }
      indexing = true;
      const found = await readIndex();
      if (found && !disposed) await begin(found, () => disposed);
      indexing = false;
      if (indexAgain && !disposed) {
        indexAgain = false;
        void readGrown();
      }
    };
    grown = throttled(() => {
      void readEvents();
      if (!off) void readGrown();
    }, GROWS_EVERY_MS);
    return () => {
      disposed = true;
      closed = true;
      cancelAnimationFrame(frame);
      grown?.stop();
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
  // The event the last step or click went to, and where it put the replay (any other seek forgets it): it stays the
  // current one, so events at the same moment are stepped through one by one.
  let stepped: { id: number; landed: number } | null = $state.raw(null);
  function seekTo(t: number, from: { id: number } | null = null) {
    if (!playback) return;
    stalled = false;
    stepped = from === null ? null : { id: from.id, landed: Math.max(0, Math.min(t, end)) };
    void playback.seek(t).then(() => (time = playback!.time));
    time = Math.max(0, Math.min(t, end));
  }
  /** The previous or the next event in the list, within what is recorded. */
  function step(direction: -1 | 1) {
    for (let i = current + direction; i >= 0 && i < marks.length; i += direction) {
      const t = marks[i]!.t;
      if (t < 0 || t > end) continue;
      return seekTo(landing(t), { id: marks[i]!.line.event.id });
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


  function save(name: string, type: string, body: string) {
    const url = URL.createObjectURL(new Blob([body], { type }));
    const a = Object.assign(document.createElement('a'), { href: url, download: name });
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  const exported = () => `easi-run-${id}-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, '')}`;
  const wallTime = (ms: number) => new Date(ms).toLocaleTimeString(i18n.locale);
  /** Everything a line says, for its tooltip: the list shows one line of it. */
  const whole = (l: (typeof said)[number]) =>
    [l.times > 1 ? `${l.text} ${say('times', { count: l.times })}` : l.text, l.outcome, l.detail, l.by, ...(l.steps ?? []), wallTime(l.event.at)].filter(Boolean).join('\n');
  const FILTERS = ['all', 'agent', 'people', 'problems'] as const;
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

  <div class="grid min-h-0 flex-1 {player ? 'grid-rows-[minmax(0,1fr)_auto] lg:grid-cols-[minmax(0,1fr)_22rem] lg:grid-rows-1' : 'grid-rows-[auto_minmax(0,1fr)]'}">
    {#if !player}
      <p class="border-b border-line bg-wash px-4 py-2.5 text-center text-[13px] text-muted sm:px-6" role="status">{off ? t('replay.off') : t('replay.none')}</p>
    {:else}
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
                  onclick={() => (speed = s)}>{s.toLocaleString(i18n.locale)}×</button
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
    {/if}

    <aside class="flex min-h-0 flex-col bg-paper {player ? 'max-h-[42vh] border-t border-line lg:max-h-none lg:border-l lg:border-t-0' : 'mx-auto w-full max-w-3xl'}" aria-labelledby="replay-events">
      <div class="shrink-0 space-y-2 border-b border-line px-3 pb-2.5 pt-3">
        <div class="flex items-center gap-2">
          <h3 id="replay-events" class="flex-1 text-sm font-semibold">{t('replay.events')} <span class="ml-1 text-[12px] font-normal tabular-nums text-muted">{rows.length}</span></h3>
          <button
            class="inline-flex h-7 items-center gap-1 rounded-full px-2 text-[12px] font-semibold text-muted transition hover:bg-wash hover:text-ink {focusRing}"
            type="button"
            title={t('timeline.export.markdown')}
            aria-label={t('timeline.export.markdown')}
            onclick={() => save(`${exported()}.md`, 'text/markdown', markdown(named || t('timeline.open'), said, wallTime))}><Icon d={ICONS.download} class="size-3.5" />MD</button
          >
          <button
            class="inline-flex h-7 items-center gap-1 rounded-full px-2 text-[12px] font-semibold text-muted transition hover:bg-wash hover:text-ink {focusRing}"
            type="button"
            title={t('timeline.export.json')}
            aria-label={t('timeline.export.json')}
            onclick={() => save(`${exported()}.json`, 'application/json', JSON.stringify({ title: named || t('timeline.open'), events }, null, 2))}><Icon d={ICONS.download} class="size-3.5" />JSON</button
          >
        </div>
        <div class="grid grid-cols-4 rounded-full bg-wash p-0.5" role="group" aria-label={t('timeline.filter')}>
          {#each FILTERS as f (f)}
            <button
              class="truncate rounded-full px-1.5 py-1 text-[12px] font-semibold transition {focusRing} {filter === f ? 'bg-paper text-ink shadow-sm' : 'text-muted hover:text-ink'}"
              type="button"
              aria-pressed={filter === f}
              onclick={() => (filter = f)}>{t(`timeline.filter.${f}`)}</button
            >
          {/each}
        </div>
        {#if fresh.length > 0}
          <p class="text-[12px] leading-snug text-muted">
            <span class="mr-1 inline-block size-1.5 rounded-full bg-accent align-middle"></span><span class="font-semibold text-ink">{left ? t('timeline.since.left', { time: wallTime(left) }) : t('timeline.since.looked')}</span>
            · {[t('timeline.count.actions', { count: news.actions }), t('timeline.count.pages', { count: news.pages }), t('timeline.count.problems', { count: news.problems })].join(' · ')}
          </p>
          {#if news.waiting > 0}<p class="text-[12px] font-semibold text-accent">{t('timeline.count.waiting', { count: news.waiting })}</p>{/if}
        {/if}
        {#if first > 1}<p class="text-[12px] text-muted">{t('timeline.dropped')}</p>{/if}
      </div>
      {#if eventsFailure}
        <p class="flex items-center gap-3 px-4 py-3 text-sm" role="alert">
          <span class="flex-1 font-semibold text-bad">{eventsFailure()}</span>
          <Button size="sm" icon={ICONS.retry} onclick={() => void readEvents()}>{t('replay.retry')}</Button>
        </p>
      {/if}
      {#if !loaded}
        <p class="px-4 py-3 text-sm text-muted">{t('timeline.loading')}</p>
      {:else if rows.length === 0}
        {#if !eventsFailure}<p class="px-4 py-3 text-sm text-muted">{t('replay.events.none')}</p>{/if}
      {:else}
        <ol class="min-h-0 flex-1 overflow-y-auto py-1">
          {#each rows as r, i (r.line.event.id)}
            {@const now = r.line.event.id === currentId}
            {@const details = [r.line.outcome, r.line.detail, r.line.by, ...(r.line.steps ?? [])].filter(Boolean)}
            <li class={now ? 'bg-accent-soft' : i % 2 === 1 ? 'bg-wash/70' : ''}>
              <button
                class="relative flex h-8 w-full items-center gap-2.5 px-4 text-left transition hover:bg-ink/[0.04] {focusRing}"
                type="button"
                title={whole(r.line)}
                aria-current={now ? 'true' : undefined}
                aria-expanded={details.length > 0 ? opened === r.line.event.id : undefined}
                onclick={() => {
                  opened = opened === r.line.event.id ? null : r.line.event.id;
                  if (r.t !== null) seekTo(landing(r.t), { id: r.line.event.id });
                }}
              >
                {#if r.line.event.seq > seen}<span class="absolute inset-y-1.5 left-1 w-0.5 rounded-full bg-accent" aria-hidden="true"></span>{/if}
                <Icon d={ACTOR_ICONS[r.line.actor]} class="size-3.5 shrink-0 {now ? 'text-accent' : 'text-muted'}" />
                <span class="min-w-0 flex-1 truncate text-[13px] {TONES[r.line.tone]}"
                  >{r.line.text}{#if r.line.times > 1}<span class="ml-1 text-muted">{say('times', { count: r.line.times })}</span>{/if}{#if r.line.outcome}<span class="ml-1.5 text-bad">· {r.line.outcome}</span>{/if}</span
                >
                <span class="shrink-0 text-[12px] tabular-nums text-muted">{r.t !== null ? clock(Math.max(0, r.t)) : wallTime(r.line.event.at)}</span>
              </button>
              {#if opened === r.line.event.id && details.length > 0}
                <div class="space-y-0.5 px-4 pb-2 pl-10 text-[12px] leading-snug text-muted">
                  {#each details as d, k (k)}<p class="break-words {k === 0 && r.line.outcome ? 'text-bad' : ''}">{d}</p>{/each}
                </div>
              {/if}
            </li>
          {/each}
        </ol>
      {/if}
    </aside>
  </div>
</dialog>
