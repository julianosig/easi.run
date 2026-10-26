<script lang="ts">
  // The administrator's bar over a session's screen, in the demo's (see Dash): the way back to the sessions (an arrow),
  // control (take it, give it back) and who holds it, a persistent session's name; then recording, the connection,
  // sound, full screen and Close session.
  import type { Snippet } from 'svelte';
  import type { SoundState } from '../audio.ts';
  import { LeaseHolder } from '../protocol.ts';
  import type { Features, ViewerStats } from '../viewer.ts';
  import Connection from './Connection.svelte';
  import Dash from './Dash.svelte';
  import Icon from './Icon.svelte';
  import Volume from './Volume.svelte';
  import { focusRing } from './ui.ts';

  let {
    holder,
    disconnected,
    status,
    name,
    features,
    stats,
    sound,
    level = $bindable(),
    onMute,
    onVolume,
    onTake,
    onGiveBack,
    onSessions,
    onClose,
    pinned = false,
    record,
    note = '',
  }: {
    holder: LeaseHolder | null;
    disconnected: boolean;
    /** Who holds control, in words. */
    status: string;
    /** A persistent session's name. */
    name: string | null;
    features: Features | null;
    stats: ViewerStats | null;
    sound: SoundState;
    level: number;
    onMute: (muted: boolean) => void;
    onVolume: (volume: number) => void;
    onTake: () => void;
    onGiveBack: () => void;
    onSessions: () => void;
    onClose: () => void;
    /** Keeps the bar shown (a recording menu open, a recording under way). */
    pinned?: boolean;
    record?: Snippet;
    /** What holding control means, on the status. */
    note?: string;
  } = $props();

  const mine = $derived(holder === LeaseHolder.You);
  const noteId = `control-note-${Math.random().toString(36).slice(2, 10)}`;
  /** The control button as the lease stands: its look, its words, and what a click does. */
  const control = $derived.by(() => {
    if (disconnected) return { tone: 'text-muted/60', title: 'Disconnected', action: null, moving: false };
    if (holder === null) return { tone: 'text-muted motion-safe:animate-pulse', title: 'Connecting…', action: null, moving: false };
    if (mine) return { tone: 'text-accent', title: note ? `You hold control · Give it back\n${note}` : 'You hold control · Give it back', action: onGiveBack, moving: false };
    if (holder === LeaseHolder.Other) return { tone: 'text-ink hover:text-accent', title: 'Another viewer holds control · Take it', action: onTake, moving: false };
    return { tone: 'text-you hover:text-accent', title: 'The agent may act · Take control', action: onTake, moving: true };
  });
  let connection = $state(false);

  // The whole page goes full screen, so the bar comes along. A page framing this one may not allow it: no button then.
  const canFullScreen = document.fullscreenEnabled;
  let fullScreen = $state(document.fullscreenElement !== null);
  function toggleFullScreen() {
    const change = document.fullscreenElement ? document.exitFullscreen() : document.documentElement.requestFullscreen();
    change.catch(() => {});
  }
</script>

<svelte:document onfullscreenchange={() => (fullScreen = document.fullscreenElement !== null)} />

<Dash label="Session controls" pinned={pinned || connection}>
  {#snippet lead()}
    <button
      class="grid size-10 place-items-center rounded-full text-ink/70 transition hover:text-accent {focusRing}"
      type="button"
      aria-label="Sessions"
      title="Sessions"
      onclick={onSessions}
    >
      <svg viewBox="0 0 24 24" class="size-5" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m15 6-6 6 6 6" /></svg>
    </button>
    <!-- Control as one bare icon: blue while you hold it; coral, the cursor drifting, while the agent may act. Hovering a
         take shows the blue it leads to. -->
    <span class="sr-only" aria-live="polite">{status}</span>
    <button
      class="control grid size-10 place-items-center rounded-full transition duration-300 disabled:cursor-default {focusRing} {control.tone}"
      type="button"
      aria-label="Hold control"
      aria-pressed={mine}
      aria-describedby={note ? noteId : undefined}
      title={control.title}
      disabled={control.action === null}
      onclick={() => control.action?.()}
    >
      <svg viewBox="0 0 24 24" class="size-6 overflow-visible" aria-hidden="true">
        <g class="cursor {control.moving ? 'moving' : ''}">
          <path d="M5 3.5l6.6 16.2 2.3-6.9 6.9-2.3z" fill="currentColor" fill-opacity=".22" stroke="currentColor" stroke-width="1.9" stroke-linejoin="round" />
          {#if mine}<circle cx="19" cy="19" r="2.6" fill="currentColor" />{/if}
          {#if disconnected}<path d="M4 20 20 4" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" />{/if}
        </g>
      </svg>
    </button>
    {#if note}<span id={noteId} class="sr-only">{note}</span>{/if}
    {#if name}
      <span class="hidden min-w-0 items-center gap-2 2xl:flex">
        <span class="truncate text-sm font-semibold">{name}</span>
        <span class="shrink-0 rounded-full bg-accent-soft px-2 py-0.5 text-xs font-semibold text-accent">Persistent</span>
      </span>
    {/if}
  {/snippet}

  {#snippet trail()}
    {#if features?.recording}
      {@render record?.()}
    {/if}
    <Connection {stats} bind:open={connection} />
    {#if features?.sound}<Volume {sound} bind:level {onMute} {onVolume} />{/if}
    {#if canFullScreen}
      <button
        class="grid size-10 place-items-center rounded-full text-muted transition hover:bg-wash hover:text-ink {focusRing}"
        type="button"
        aria-label={fullScreen ? 'Exit full screen' : 'Full screen'}
        title={fullScreen ? 'Exit full screen' : 'Full screen'}
        onclick={toggleFullScreen}
      >
        <Icon class="size-5" d={fullScreen ? 'M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5' : 'M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5'} />
      </button>
    {/if}
    <button
      class="ml-1 grid size-10 place-items-center rounded-full text-muted transition hover:bg-bad/10 hover:text-bad {focusRing}"
      type="button"
      aria-label="Close session"
      title="Close session"
      onclick={onClose}
    >
      <svg viewBox="0 0 24 24" class="size-5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3.5v8M7.05 6.4a7.5 7.5 0 1 0 9.9 0" /></svg>
    </button>
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
