<script lang="ts">
  // The demo visitor's one bar, floating over the screen (see Dash): the demos, control (with a view-only switch), the
  // connection, sound, time left and Leave.
  import type { Snippet } from 'svelte';
  import type { SoundState } from '../audio.ts';
  import { LeaseHolder } from '../protocol.ts';
  import type { ViewerStats } from '../viewer.ts';
  import Connection from './Connection.svelte';
  import Dash from './Dash.svelte';
  import Mark from './Mark.svelte';
  import Volume from './Volume.svelte';
  import { demoOf } from './demos.ts';
  import { dismissable } from './dismiss.ts';
  import { focusRing } from './ui.ts';

  let {
    sites,
    site,
    switching,
    onPick,
    holder,
    autoFailed,
    viewOnly,
    onViewOnly,
    onTake,
    stats,
    sound,
    level = $bindable(),
    onMute,
    onVolume,
    left,
    total,
    onLeave,
    pinned = false,
    picker = true,
    record,
  }: {
    sites: string[];
    site: string;
    /** Whether the bar lists the demos (a page framing the demo lists them itself). */
    picker?: boolean;
    switching: string | null;
    onPick: (name: string) => void;
    holder: LeaseHolder | null;
    autoFailed: boolean;
    viewOnly: boolean;
    onViewOnly: (on: boolean) => void;
    onTake: () => void;
    stats: ViewerStats | null;
    sound: SoundState;
    level: number;
    onMute: (muted: boolean) => void;
    onVolume: (volume: number) => void;
    left: number;
    total: number;
    onLeave: () => void;
    /** Keeps the bar shown (a recording menu open, a recording under way). */
    pinned?: boolean;
    /** The recording control, shown before the connection. */
    record?: Snippet;
  } = $props();

  const mine = $derived(holder === LeaseHolder.You);
  const current = $derived(demoOf(switching ?? site));

  let menu = $state(false);
  let details = $state(false);
  let menuRoot: HTMLDivElement | undefined = $state();

  $effect(() => {
    if (menu) return dismissable(menuRoot, () => (menu = false));
  });

  /** `keyboard`: the click came from Enter or Space, which a click count of 0 tells apart from a pointer's. */
  function pick(name: string, keyboard: boolean) {
    menu = false;
    // The list goes away with the focused item: from the keyboard, focus returns to its button. A pointer's pick moves
    // no focus (a focused button would read as keyboard focus to some browsers and hold the bar out).
    if (keyboard) menuRoot?.querySelector('button')?.focus();
    if (name !== (switching ?? site)) onPick(name);
  }

  const status = $derived(
    holder === null
      ? 'Connecting…'
      : mine
        ? viewOnly
          ? 'You’re watching'
          : 'You’re in control'
        : holder === LeaseHolder.Other
          ? 'Another tab is in control'
          : autoFailed
            ? 'Could not take control'
            : 'Taking control…',
  );
  // Minutes and seconds from a minute up, seconds below.
  const clock = $derived(left >= 60 ? `${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}` : String(left));
</script>

<Dash label="Demo controls" pinned={pinned || menu || details || switching !== null}>
  <!-- Two zones: who, where and control on the left; the session on the right. -->
  {#snippet lead()}
    <p class="flex shrink-0 items-center">
      <Mark class="h-[24.5px] w-auto" /><span class="sr-only">easi.run</span>
    </p>

    {#if picker}
    <div class="relative" bind:this={menuRoot}>
      <button
        class="flex items-center gap-2 rounded-xl border border-line bg-wash py-2 pl-3 pr-2.5 text-sm font-semibold transition hover:border-ink/25 {focusRing}"
        type="button"
        aria-expanded={menu}
        aria-controls="demo-list"
        onclick={() => (menu = !menu)}
      >
        <svg viewBox="0 0 24 24" class="size-4 text-accent" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={current.icon} /></svg>
        <span>{current.title}</span>
        <svg viewBox="0 0 24 24" class="size-4 text-muted transition {menu ? 'rotate-180' : ''}" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6" /></svg>
      </button>
      {#if menu}
        <ul id="demo-list" class="absolute left-0 top-full z-30 mt-2 w-80 list-none rounded-2xl border border-line bg-paper p-1.5 shadow-[0_24px_60px_-24px_rgb(20_40_75/0.45)]" aria-label="Demos">
          {#each sites as name (name)}
            {@const d = demoOf(name)}
            <li>
            <button
              class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition hover:bg-wash {name === site ? 'bg-accent-soft' : ''} {focusRing}"
              type="button"
              aria-current={name === site ? 'true' : undefined}
              onclick={(e) => pick(name, e.detail === 0)}
            >
              <svg viewBox="0 0 24 24" class="size-5 shrink-0 text-accent" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={d.icon} /></svg>
              <span class="min-w-0 flex-1">
                <span class="block text-sm font-semibold">{d.title}</span>
                <span class="block truncate text-xs text-muted">{d.line}</span>
              </span>
              {#if name === site}<svg viewBox="0 0 24 24" class="size-4 text-accent" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m5 12 5 5 9-10" /></svg>{/if}
            </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
    {/if}
    {#if mine}
      <!-- Holding control: one lock. Open (blue), the page takes your input; closed (red), you only watch. -->
      <span class="sr-only" aria-live="polite">{status}</span>
      <button
        class="group grid size-11 place-items-center rounded-full border transition {viewOnly ? 'border-bad/40 bg-bad/10 text-bad hover:bg-bad/15' : 'border-accent/40 bg-accent-soft text-accent hover:border-accent/60'} {focusRing}"
        type="button"
        role="switch"
        aria-checked={viewOnly}
        aria-label="View only"
        title={viewOnly ? 'View only: unlock to use the page' : 'You’re in control: lock to only watch'}
        onclick={() => onViewOnly(!viewOnly)}
      >
        <svg viewBox="0 0 24 24" class="size-6 overflow-visible" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path class="lock-shackle {viewOnly ? '' : 'lock-open'}" d="M8 11V7.5a4 4 0 0 1 8 0V11" />
          <rect x="5" y="11" width="14" height="10" rx="2.5" fill="currentColor" fill-opacity=".18" />
          <circle cx="12" cy="16" r="1.3" fill="currentColor" stroke="none" />
        </svg>
      </button>
    {:else}
      <div class="flex min-h-10 items-center gap-2.5 rounded-full border border-line bg-wash py-1 pl-3.5 text-sm font-semibold transition {holder === LeaseHolder.Other || autoFailed ? 'pr-1' : 'pr-3.5'}">
        <!-- Coral while the agent may act, neutral otherwise; the take leads to the blue of holding control. -->
        <span class="relative size-2.5 rounded-full {holder === LeaseHolder.Agent ? 'bg-you' : holder === null ? 'bg-muted/40' : 'bg-muted'}" aria-hidden="true"></span>
        <span aria-live="polite">{status}</span>
        {#if holder === LeaseHolder.Other || autoFailed}
          <button class="flex items-center gap-1.5 rounded-full bg-accent px-3.5 py-1.5 text-white shadow-[0_6px_16px_-6px_rgb(36_87_245/0.9)] transition hover:-translate-y-px {focusRing}" type="button" onclick={onTake}>
            <svg viewBox="0 0 24 24" class="size-4" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 3l7 17 2.5-7.5L21 10z" fill="currentColor" fill-opacity=".25" /></svg>
            {holder === LeaseHolder.Other ? 'Take control here' : 'Try again'}
          </button>
        {/if}
      </div>
    {/if}
  {/snippet}

  {#snippet trail()}
    {@render record?.()}
    {#if record}<span class="mx-1 h-7 w-px bg-line" aria-hidden="true"></span>{/if}
    <Connection {stats} bind:open={details} />
    <Volume {sound} bind:level {onMute} {onVolume} />
    <span class="mx-1 h-7 w-px bg-line" aria-hidden="true"></span>
    <span class="relative grid size-11 place-items-center" role="img" aria-label="{left} seconds left" title="{left} seconds left">
      <svg viewBox="0 0 44 44" class="absolute inset-0 -rotate-90" aria-hidden="true">
        <circle cx="22" cy="22" r="19" fill="none" class="stroke-line" stroke-width="3.5" />
        <circle cx="22" cy="22" r="19" fill="none" class="transition-[stroke-dashoffset] duration-300 {left <= 10 ? 'stroke-bad' : 'stroke-accent'}" stroke-width="3.5" stroke-linecap="round" stroke-dasharray="119.4" stroke-dashoffset={119.4 * (1 - left / total)} />
      </svg>
      <span class="text-[12px] font-bold tabular-nums {left <= 10 ? 'text-bad' : 'text-ink'}" aria-hidden="true">{clock}</span>
    </span>
    <button class="ml-1 flex items-center gap-1.5 rounded-xl px-3 py-2 text-sm font-semibold text-muted transition hover:bg-bad/10 hover:text-bad {focusRing}" type="button" onclick={onLeave}>
      <svg viewBox="0 0 24 24" class="size-[18px]" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 4h3a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-3" /><path d="M10 16l-4-4 4-4M6 12h10" /></svg>
      Leave
    </button>
  {/snippet}
</Dash>

<style>
  /* The shackle swings up and out of the body when the lock opens. */
  .lock-shackle {
    transform-box: view-box;
    transform-origin: 16px 11px;
    transition: transform 0.35s cubic-bezier(0.34, 1.56, 0.64, 1);
  }
  .lock-open {
    transform: translateY(-2.5px) rotate(28deg);
  }
  @media (prefers-reduced-motion: reduce) {
    .lock-shackle {
      transition: none;
    }
  }
</style>
