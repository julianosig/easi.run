<script lang="ts">
  // The dashbar's recording control: a Record button that opens its own menu (microphone, camera, the page's sound)
  // and starts the recording; while recording, a red REC timer that stops it. With a camera on, the menu also picks its
  // corner radius and border; the camera is placed and sized on the screen itself (CameraBubble).
  import { type CameraBorder, type CameraLayout, type Devices, microphoneSupported } from '../recorder.ts';
  import { dismissable } from './dismiss.ts';
  import { focusRing } from './ui.ts';

  let {
    open = $bindable(false),
    devices,
    micId,
    camId,
    pageSound = $bindable(),
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

  const BORDERS: [CameraBorder, string][] = [['white', 'White'], ['coral', 'Coral'], ['none', 'None']];

  let root: HTMLDivElement | undefined = $state();
  // Without it the microphone stays off: a recording would be told of one and get nothing.
  const micSupported = microphoneSupported();
  const clock = $derived(`${Math.floor(elapsed / 60)}:${String(elapsed % 60).padStart(2, '0')}`);
  const select = `w-full rounded-xl border border-line bg-paper px-3 py-2 text-sm ${focusRing}`;

  $effect(() => {
    if (open) return dismissable(root, () => (open = false));
  });
</script>

<div class="relative" bind:this={root}>
  {#if recording && stopping}
    <span class="flex items-center gap-2 rounded-full bg-wash px-3.5 py-2 text-sm font-semibold text-muted" role="status">
      <span class="size-3.5 animate-spin rounded-full border-2 border-muted border-t-transparent" aria-hidden="true"></span>
      Saving…
    </span>
  {:else if recording}
    <button
      class="flex items-center gap-2 rounded-full bg-bad px-3.5 py-2 text-sm font-bold tabular-nums text-white transition hover:brightness-110 {focusRing}"
      type="button"
      onclick={onStop}
      aria-label="Stop recording ({clock})"
    >
      <span class="size-2.5 animate-pulse rounded-full bg-white" aria-hidden="true"></span>
      REC {clock}
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
      Record
    </button>
  {/if}

  {#if open && !recording}
    <div id="record-menu" class="absolute right-0 top-full z-30 mt-2 w-80 rounded-2xl border border-line bg-paper p-4 shadow-[0_24px_60px_-24px_rgb(20_40_75/0.45)]">
      <p class="text-sm font-semibold">Recording</p>
      <p class="mt-1 text-sm text-muted">
        Recorded on the server in high quality, with your camera and microphone, and saved to your Downloads as it
        goes. It shows the whole screen, even what the agent sees as [redacted].
      </p>

      <label class="mt-4 block text-xs font-semibold text-muted" for="record-mic">Microphone</label>
      <select id="record-mic" class="{select} mt-1" value={micId} disabled={busy || !micSupported} onchange={(e) => onMic(e.currentTarget.value)}>
        <option value="">Off</option>
        {#if !devices.microphones.some((d) => d.id === 'default')}<option value="default">Default microphone</option>{/if}
        {#each devices.microphones as d (d.id)}<option value={d.id}>{d.label}</option>{/each}
      </select>
      {#if !micSupported}<p class="mt-1 text-xs text-muted">This browser cannot record your microphone; Chrome or Edge can.</p>{/if}

      <label class="mt-3 block text-xs font-semibold text-muted" for="record-camera">Camera</label>
      <select id="record-camera" class="{select} mt-1" value={camId} disabled={busy} onchange={(e) => onCamera(e.currentTarget.value)}>
        <option value="">Off</option>
        {#if !devices.cameras.some((d) => d.id === 'default')}<option value="default">Default camera</option>{/if}
        {#each devices.cameras as d (d.id)}<option value={d.id}>{d.label}</option>{/each}
      </select>

      {#if camId}
        <div class="mt-3 flex items-center gap-3">
          <label class="w-12 shrink-0 text-xs font-semibold text-muted" for="record-radius">Radius</label>
          <input
            id="record-radius"
            class="min-w-0 flex-1 accent-accent"
            type="range"
            min="0"
            max="100"
            step="1"
            value={layout.radius}
            aria-valuetext="{layout.radius}%"
            oninput={(e) => onLayout({ ...layout, radius: Number(e.currentTarget.value) })}
          />
          <output class="w-10 shrink-0 text-right text-xs font-semibold tabular-nums" for="record-radius">{layout.radius}%</output>
        </div>
        <div class="mt-3 flex items-center gap-3" role="group" aria-label="Camera border">
          <span class="w-12 shrink-0 text-xs font-semibold text-muted">Border</span>
          <div class="grid flex-1 grid-cols-3 gap-1 rounded-xl bg-wash p-1">
            {#each BORDERS as [value, label] (value)}
              {@const on = layout.border === value}
              <button
                class="rounded-lg px-2 py-1.5 text-xs font-semibold transition {on ? 'bg-paper text-ink shadow-sm' : 'text-muted hover:text-ink'} {focusRing}"
                type="button"
                aria-pressed={on}
                onclick={() => onLayout({ ...layout, border: value })}>{label}</button
              >
            {/each}
          </div>
        </div>
        <p class="mt-2 text-xs text-muted">Drag your camera on the screen to place it, and its corner handle to resize it.</p>
      {/if}

      <label class="mt-3 flex items-center gap-2 text-sm">
        <input class="accent-accent" type="checkbox" bind:checked={pageSound} /> Include the page’s sound
      </label>

      <button
        class="mt-4 flex w-full items-center justify-center gap-2 rounded-xl bg-bad px-4 py-2.5 text-sm font-semibold text-white transition hover:brightness-110 disabled:opacity-60 {focusRing}"
        type="button"
        disabled={busy}
        onclick={onStart}
      >
        <span class="size-2.5 rounded-full bg-white" aria-hidden="true"></span>
        Start recording
      </button>
    </div>
  {/if}
</div>
