<script lang="ts">
  // The one bar over a session's screen (see Dash), the administrator's and the demo visitor's: on the left the way
  // back to the sessions (or the demos, in the demo), control as one bare cursor, and a persistent session's name; on
  // the right recording, the connection, sound, time left (the demo), full screen and the way out (Close session, or
  // Leave in the demo).
  import type { Snippet } from 'svelte';
  import type { SoundState } from '../audio.ts';
  import { t } from '../i18n/index.svelte.ts';
  import { LeaseHolder } from '../protocol.ts';
  import type { ViewerStats } from '../viewer.ts';
  import Connection from './Connection.svelte';
  import Dash from './Dash.svelte';
  import Icon from './Icon.svelte';
  import Volume from './Volume.svelte';
  import { demoOf } from './demos.ts';
  import { dismissable } from './dismiss.ts';
  import { focusRing } from './ui.ts';

  /** What only the demo's bar has: its demos, the visitor's view-only switch, and the time left. */
  interface Demo {
    sites: string[];
    site: string;
    switching: string | null;
    onPick: (name: string) => void;
    /** Whether the bar lists the demos (a page framing the demo lists them itself). */
    picker: boolean;
    autoFailed: boolean;
    viewOnly: boolean;
    onViewOnly: (on: boolean) => void;
    left: number;
    total: number;
    onLeave: () => void;
  }

  let {
    holder,
    disconnected = false,
    stats,
    sound,
    level = $bindable(),
    onMute,
    onVolume,
    onTake,
    pinned = false,
    record,
    recording = true,
    hearing = true,
    onGiveBack,
    note = '',
    name = null,
    onSessions,
    onClose,
    demo,
  }: {
    holder: LeaseHolder | null;
    disconnected?: boolean;
    stats: ViewerStats | null;
    sound: SoundState;
    level: number;
    onMute: (muted: boolean) => void;
    onVolume: (volume: number) => void;
    onTake: () => void;
    /** Keeps the bar shown (a recording menu open, a recording under way). */
    pinned?: boolean;
    /** The recording control, shown when `recording` (the session's feature) is on. */
    record?: Snippet;
    recording?: boolean;
    /** Whether the session has sound. */
    hearing?: boolean;
    /** The administrator's: giving control back, what holding it means (on the status), a persistent session's name,
     * the way back to the sessions and Close session. */
    onGiveBack?: () => void;
    note?: string;
    name?: string | null;
    onSessions?: (() => void) | undefined;
    onClose?: (() => void) | undefined;
    /** The demo's; its bar has no way back and leaves with Leave. */
    demo?: Demo | undefined;
  } = $props();

  const mine = $derived(holder === LeaseHolder.You);
  const noteId = `control-note-${Math.random().toString(36).slice(2, 10)}`;
  /** The control button as the lease stands: who holds control in words (announced as it changes), its look, its
   * title, what a click does, and whether it is pressed (the page takes this viewer's input). */
  const control = $derived.by(() => {
    const still = { moving: false };
    if (disconnected) {
      const said = t('session.control.disconnected');
      return { ...still, said, tone: 'text-muted/60', title: said, action: null, pressed: false };
    }
    if (holder === null) {
      const said = t('session.control.connecting');
      return { ...still, said, tone: 'text-muted motion-safe:animate-pulse', title: said, action: null, pressed: false };
    }
    if (mine && demo) {
      const watching = demo.viewOnly;
      return watching
        ? { ...still, said: t('session.control.watching'), tone: 'text-ink hover:text-accent', title: t('session.control.watching.title'), action: () => demo.onViewOnly(false), pressed: false }
        : { ...still, said: t('session.control.inControl'), tone: 'text-accent', title: t('session.control.inControl.title'), action: () => demo.onViewOnly(true), pressed: true };
    }
    if (mine) {
      const title = note ? t('session.control.yours.titleNote', { note }) : t('session.control.yours.title');
      return { ...still, said: t('session.control.yours'), tone: 'text-accent', title, action: onGiveBack ?? null, pressed: true };
    }
    if (holder === LeaseHolder.Other) {
      const said = t(demo ? 'session.control.otherTab' : 'session.control.otherViewer');
      const title = t(demo ? 'session.control.otherTab.title' : 'session.control.otherViewer.title');
      return { ...still, said, tone: 'text-ink hover:text-accent', title, action: onTake, pressed: false };
    }
    if (demo?.autoFailed) return { ...still, said: t('session.control.failed'), tone: 'text-bad hover:text-accent', title: t('session.control.failed.title'), action: onTake, pressed: false };
    if (demo) {
      const said = t('session.control.taking');
      return { ...still, said, tone: 'text-muted motion-safe:animate-pulse', title: said, action: null, pressed: false };
    }
    return { said: t('session.control.agent'), tone: 'text-you hover:text-accent', title: t('session.control.agent.title'), action: onTake, pressed: false, moving: true };
  });
  let connection = $state(false);

  // The demos' list.
  let menu = $state(false);
  let menuRoot: HTMLDivElement | undefined = $state();
  const current = $derived(demo ? demoOf(demo.switching ?? demo.site) : null);
  $effect(() => {
    if (menu) return dismissable(menuRoot, () => (menu = false));
  });
  /** `keyboard`: the click came from Enter or Space, which a click count of 0 tells apart from a pointer's. */
  function pick(site: string, keyboard: boolean) {
    menu = false;
    // The list goes away with the focused item: from the keyboard, focus returns to its button. A pointer's pick moves
    // no focus (a focused button would read as keyboard focus to some browsers and hold the bar out).
    if (keyboard) menuRoot?.querySelector('button')?.focus();
    if (demo && site !== (demo.switching ?? demo.site)) demo.onPick(site);
  }
  // Minutes and seconds from a minute up, seconds below.
  const clock = $derived(!demo ? '' : demo.left >= 60 ? `${Math.floor(demo.left / 60)}:${String(demo.left % 60).padStart(2, '0')}` : String(demo.left));

  // The whole page goes full screen, so the bar comes along. A page framing this one may not allow it: no button then.
  const canFullScreen = document.fullscreenEnabled;
  let fullScreen = $state(document.fullscreenElement !== null);
  function toggleFullScreen() {
    const change = document.fullscreenElement ? document.exitFullscreen() : document.documentElement.requestFullscreen();
    change.catch(() => {});
  }
</script>

<svelte:document onfullscreenchange={() => (fullScreen = document.fullscreenElement !== null)} />

<Dash label={t(demo ? 'session.bar.demo' : 'session.bar.session')} pinned={pinned || connection || menu || (demo?.switching ?? null) !== null}>
  {#snippet lead()}
    {#if onSessions}
      <button
        class="grid size-10 place-items-center rounded-full text-ink/70 transition hover:text-accent {focusRing}"
        type="button"
        aria-label={t('session.back')}
        title={t('session.back')}
        onclick={onSessions}
      >
        <Icon d="m15 6-6 6 6 6" class="size-5" stroke={2.2} />
      </button>
    {/if}
    {#if demo?.picker && current}
      <div class="relative" bind:this={menuRoot}>
        <button
          class="flex items-center gap-2 rounded-xl border border-line bg-wash py-2 pl-3 pr-2.5 text-sm font-semibold transition hover:border-ink/25 {focusRing}"
          type="button"
          aria-expanded={menu}
          aria-controls="demo-list"
          onclick={() => (menu = !menu)}
        >
          <Icon d={current.icon} class="size-4 text-accent" stroke={1.9} />
          <span>{t(current.title)}</span>
          <Icon d="m6 9 6 6 6-6" class="size-4 text-muted transition {menu ? 'rotate-180' : ''}" stroke={2.2} />
        </button>
        {#if menu}
          <ul id="demo-list" class="absolute left-0 top-full z-30 mt-2 w-80 list-none rounded-2xl border border-line bg-paper p-1.5 shadow-[0_24px_60px_-24px_rgb(20_40_75/0.45)]" aria-label={t('session.demos')}>
            {#each demo.sites as site (site)}
              {@const d = demoOf(site)}
              <li>
                <button
                  class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition hover:bg-wash {site === demo.site ? 'bg-accent-soft' : ''} {focusRing}"
                  type="button"
                  aria-current={site === demo.site ? 'true' : undefined}
                  onclick={(e) => pick(site, e.detail === 0)}
                >
                  <Icon d={d.icon} class="size-5 shrink-0 text-accent" />
                  <span class="min-w-0 flex-1">
                    <span class="block text-sm font-semibold">{t(d.title)}</span>
                    <span class="block truncate text-xs text-muted">{t(d.line)}</span>
                  </span>
                  {#if site === demo.site}<Icon d="m5 12 5 5 9-10" class="size-4 text-accent" stroke={2.5} />{/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
    <!-- Control as one bare icon: blue while this viewer's input reaches the page; coral, the cursor drifting, while the
         agent may act. Hovering a take shows the blue it leads to. -->
    <span class="sr-only" aria-live="polite">{control.said}</span>
    <button
      class="control grid size-10 place-items-center rounded-full transition duration-300 disabled:cursor-default {focusRing} {control.tone}"
      type="button"
      aria-label={t('session.control')}
      aria-pressed={control.pressed}
      aria-describedby={note ? noteId : undefined}
      title={control.title}
      disabled={control.action === null}
      onclick={() => control.action?.()}
    >
      <svg viewBox="0 0 24 24" class="size-6 overflow-visible" aria-hidden="true">
        <g class="cursor {control.moving ? 'moving' : ''}">
          <path d="M5 3.5l6.6 16.2 2.3-6.9 6.9-2.3z" fill="currentColor" fill-opacity=".22" stroke="currentColor" stroke-width="1.9" stroke-linejoin="round" />
          {#if control.pressed}<circle cx="19" cy="19" r="2.6" fill="currentColor" />{/if}
          {#if disconnected}<path d="M4 20 20 4" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" />{/if}
        </g>
      </svg>
    </button>
    {#if note}<span id={noteId} class="sr-only">{note}</span>{/if}
    {#if name}
      <span class="hidden min-w-0 items-center gap-2 2xl:flex">
        <span class="truncate text-sm font-semibold">{name}</span>
        <span class="shrink-0 rounded-full bg-accent-soft px-2 py-0.5 text-xs font-semibold text-accent">{t('session.persistent')}</span>
      </span>
    {/if}
  {/snippet}

  {#snippet trail()}
    {#if recording}
      {@render record?.()}
    {/if}
    <Connection {stats} bind:open={connection} />
    {#if hearing}<Volume {sound} bind:level {onMute} {onVolume} />{/if}
    {#if demo}
      <span class="relative grid size-10 place-items-center" role="img" aria-label={t('session.timeLeft', { count: demo.left })} title={t('session.timeLeft', { count: demo.left })}>
        <svg viewBox="0 0 44 44" class="absolute inset-0 -rotate-90" aria-hidden="true">
          <circle cx="22" cy="22" r="19" fill="none" class="stroke-line" stroke-width="3.5" />
          <circle cx="22" cy="22" r="19" fill="none" class="transition-[stroke-dashoffset] duration-300 {demo.left <= 10 ? 'stroke-bad' : 'stroke-accent'}" stroke-width="3.5" stroke-linecap="round" stroke-dasharray="119.4" stroke-dashoffset={119.4 * (1 - demo.left / demo.total)} />
        </svg>
        <span class="text-[12px] font-bold tabular-nums {demo.left <= 10 ? 'text-bad' : 'text-ink'}" aria-hidden="true">{clock}</span>
      </span>
    {/if}
    {#if canFullScreen}
      <button
        class="grid size-10 place-items-center rounded-full text-muted transition hover:bg-wash hover:text-ink {focusRing}"
        type="button"
        aria-label={t(fullScreen ? 'session.fullScreen.exit' : 'session.fullScreen')}
        title={t(fullScreen ? 'session.fullScreen.exit' : 'session.fullScreen')}
        onclick={toggleFullScreen}
      >
        <Icon class="size-5" d={fullScreen ? 'M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5' : 'M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5'} />
      </button>
    {/if}
    {#if demo}
      <button class="ml-1 flex items-center gap-1.5 rounded-xl px-3 py-2 text-sm font-semibold text-muted transition hover:bg-bad/10 hover:text-bad {focusRing}" type="button" onclick={demo.onLeave}>
        <Icon d="M15 4h3a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-3M10 16l-4-4 4-4M6 12h10" stroke={2} />
        {t('session.leave')}
      </button>
    {:else if onClose}
      <button
        class="ml-1 grid size-10 place-items-center rounded-full text-muted transition hover:bg-bad/10 hover:text-bad {focusRing}"
        type="button"
        aria-label={t('session.close')}
        title={t('session.close')}
        onclick={onClose}
      >
        <Icon d="M12 3.5v8M7.05 6.4a7.5 7.5 0 1 0 9.9 0" class="size-5" stroke={2} />
      </button>
    {/if}
  {/snippet}
</Dash>

<style>
  /* The agent's cursor drifts as on the landing; yours sits still. A change of hand eases the cursor in. */
  .cursor {
    transform-box: view-box;
    transform-origin: 12px 12px;
    transition: transform 0.3s cubic-bezier(0.34, 1.56, 0.64, 1);
  }
  .control[aria-pressed='true'] .cursor {
    transform: scale(1.06);
  }
  .moving {
    animation: drift 2.6s ease-in-out infinite;
  }
  @keyframes drift {
    0%,
    100% {
      transform: translate(0, 0);
    }
    35% {
      transform: translate(1.6px, 1.2px);
    }
    70% {
      transform: translate(-0.8px, 1.8px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .cursor,
    .moving {
      animation: none;
      transition: none;
    }
  }
</style>
