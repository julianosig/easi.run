<script lang="ts">
  // The dashbar's recording control: a Record button that opens its own menu (microphone, camera, the page's sound)
  // and starts the recording; while recording, a red REC timer that stops it. With a camera on, the menu also picks its
  // corner radius; the camera is placed and sized on the screen itself (CameraBubble).
  import { t } from '../i18n/index.svelte.ts';
  import { type CameraLayout, type Devices, microphoneSupported } from '../recorder.ts';
  import { dismissable } from './dismiss.ts';
  import Icon from './Icon.svelte';
  import { focusRing } from './ui.ts';

  let {
    open = $bindable(false),
    devices,
    micId,
    camId,
    pageSound = $bindable(),
    sound,
    layout,
    onLayout,
    recording,
    stopping = false,
    elapsed,
    busy,
    onMic,
    onCamera,
    onStart,
    onStop,
  }: {
    open?: boolean;
    devices: Devices;
    micId: string;
    camId: string;
    pageSound: boolean;
    /** Whether the session has its page's sound, which a recording can then include. */
    sound: boolean;
    /** Where and how the camera sits in the recording. */
    layout: CameraLayout;
    onLayout: (layout: CameraLayout) => void;
    recording: boolean;
    /** Stopped, and finishing the file. */
    stopping?: boolean;
    elapsed: number;
    busy: boolean;
    onMic: (id: string) => void;
    onCamera: (id: string) => void;
    onStart: () => void;
    onStop: () => void;
  } = $props();

  /** A speaker with its sound. */
  const SOUND = 'M11 5 6 9H3v6h3l5 4zM15.5 8.5a5 5 0 0 1 0 7M18.5 5.5a9 9 0 0 1 0 13';

  let root: HTMLDivElement | undefined = $state();
  // Without it the microphone stays off: a recording would be told of one and get nothing.
  const micSupported = microphoneSupported();
  const clock = $derived(`${Math.floor(elapsed / 60)}:${String(elapsed % 60).padStart(2, '0')}`);
  /** Drawn without the browser's arrow, which sits against the edge: the menu draws its own, inset as the text is. */
  const select = `w-full appearance-none rounded-xl border border-line bg-paper py-2 pl-3 pr-10 text-sm ${focusRing}`;

  $effect(() => {
    if (open) return dismissable(root, () => (open = false));
  });
</script>

{#snippet chevron()}
  <Icon d="m6 9 6 6 6-6" class="pointer-events-none absolute right-3 top-1/2 size-4 -translate-y-1/2 text-muted" stroke={2} />
{/snippet}

<div class="relative" bind:this={root}>
  {#if recording && stopping}
    <span class="flex items-center gap-2 rounded-full bg-wash px-3.5 py-2 text-sm font-semibold text-muted" role="status">
      <span class="size-3.5 animate-spin rounded-full border-2 border-muted border-t-transparent" aria-hidden="true"></span>
      {t('session.record.saving')}
    </span>
  {:else if recording}
    <button
      class="flex items-center gap-2 rounded-full bg-bad px-3.5 py-2 text-sm font-bold tabular-nums text-white transition hover:brightness-110 {focusRing}"
      type="button"
      onclick={onStop}
      aria-label={t('session.record.stop', { time: clock })}
    >
      <span class="size-2.5 animate-pulse rounded-full bg-white" aria-hidden="true"></span>
      {t('session.record.rec', { time: clock })}
      <span class="ml-1 grid size-5 place-items-center rounded bg-white/20" aria-hidden="true"><span class="size-2 rounded-sm bg-white"></span></span>
    </button>
  {:else}
    <button
      class="flex items-center gap-2 rounded-full px-3 py-2 text-sm font-semibold transition hover:bg-wash {open ? 'bg-wash' : ''} {focusRing}"
      type="button"
      aria-expanded={open}
      aria-controls="record-menu"
      onclick={() => (open = !open)}
    >
      <span class="grid size-4 place-items-center rounded-full ring-2 ring-bad" aria-hidden="true"><span class="size-2 rounded-full bg-bad"></span></span>
      {t('session.record.open')}
    </button>
  {/if}

  {#if open && !recording}
    <div id="record-menu" class="absolute right-0 top-full z-30 mt-2 w-80 rounded-2xl border border-line bg-paper p-4 shadow-[0_24px_60px_-24px_rgb(20_40_75/0.45)]">
      <p class="text-sm font-semibold">{t('session.record.title')}</p>
      <p class="mt-1 text-sm text-muted">{t('session.record.help')}</p>

      <label class="mt-4 block text-xs font-semibold text-muted" for="record-mic">{t('session.record.microphone')}</label>
      <div class="relative mt-1">
        <select id="record-mic" class={select} value={micId} disabled={busy || !micSupported} onchange={(e) => onMic(e.currentTarget.value)}>
          <option value="">{t('session.record.off')}</option>
          {#if !devices.microphones.some((d) => d.id === 'default')}<option value="default">{t('session.record.microphone.default')}</option>{/if}
          {#each devices.microphones as d (d.id)}<option value={d.id}>{d.label ?? t('session.record.microphone.numbered', { number: d.number })}</option>{/each}
        </select>
        {@render chevron()}
      </div>
      {#if !micSupported}<p class="mt-1 text-xs text-muted">{t('session.record.microphone.unsupported')}</p>{/if}

      <label class="mt-3 block text-xs font-semibold text-muted" for="record-camera">{t('session.record.camera')}</label>
      <div class="relative mt-1">
        <select id="record-camera" class={select} value={camId} disabled={busy} onchange={(e) => onCamera(e.currentTarget.value)}>
          <option value="">{t('session.record.off')}</option>
          {#if !devices.cameras.some((d) => d.id === 'default')}<option value="default">{t('session.record.camera.default')}</option>{/if}
          {#each devices.cameras as d (d.id)}<option value={d.id}>{d.label ?? t('session.record.camera.numbered', { number: d.number })}</option>{/each}
        </select>
        {@render chevron()}
      </div>

      {#if camId}
        <div class="mt-3 flex items-center gap-3">
          <label class="w-12 shrink-0 text-xs font-semibold text-muted" for="record-radius">{t('session.record.radius')}</label>
          <input
            id="record-radius"
            class="min-w-0 flex-1 accent-accent"
            type="range"
            min="0"
            max="100"
            step="1"
            value={layout.radius}
            aria-valuetext={t('session.percent', { percent: layout.radius })}
            oninput={(e) => onLayout({ ...layout, radius: Number(e.currentTarget.value) })}
          />
          <output class="w-10 shrink-0 text-right text-xs font-semibold tabular-nums" for="record-radius">{t('session.percent', { percent: layout.radius })}</output>
        </div>
        <p class="mt-2 text-xs text-muted">{t('session.record.place')}</p>
      {/if}

      {#if sound}
        <label class="mt-3 flex items-center gap-2.5 text-sm">
          <Icon d={SOUND} class="size-[18px] shrink-0 text-accent" />
          <span class="flex-1">{t('session.record.pageSound')}</span>
          <input class="size-4 accent-accent" type="checkbox" bind:checked={pageSound} />
        </label>
      {/if}

      <button
        class="mt-4 flex w-full items-center justify-center gap-2 rounded-xl bg-bad px-4 py-2.5 text-sm font-semibold text-white transition hover:brightness-110 disabled:opacity-60 {focusRing}"
        type="button"
        disabled={busy}
        onclick={onStart}
      >
        <span class="size-2.5 rounded-full bg-white" aria-hidden="true"></span>
        {t('session.record.start')}
      </button>
    </div>
  {/if}
</div>
