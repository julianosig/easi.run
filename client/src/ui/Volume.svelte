<script lang="ts">
  // The sound control: a mute button and a volume slider (0-100), both acting here only (the browser itself plays
  // on). The level is remembered; the session applies it when its viewer starts. Without sound support the button
  // stays focusable so keyboard users still hear why.
  import type { SoundState } from '../audio.ts';
  import { remember } from './api.ts';
  import { focusRing } from './ui.ts';

  let {
    sound,
    level = $bindable(),
    onMute,
    onVolume,
  }: { sound: SoundState; level: number; onMute: (muted: boolean) => void; onVolume: (volume: number) => void } = $props();

  let muted = $state(false);
  const silent = $derived(muted || level === 0);
  const off = $derived(sound === 'unsupported');

  function toggle() {
    muted = !muted;
    onMute(muted);
  }
  function slide(value: number) {
    level = value;
    remember('volume', String(value));
    onVolume(value / 100);
    if (muted && value > 0) toggle();
  }
</script>

<div class="flex items-center gap-1 rounded-full px-1 transition hover:bg-wash">
  <button
    class="grid size-9 place-items-center rounded-full text-ink hover:text-accent {focusRing} aria-disabled:opacity-40"
    type="button"
    onclick={() => off || toggle()}
    aria-disabled={off}
    aria-label={off ? 'This browser cannot play the sound' : muted ? 'Unmute' : 'Mute'}
    title={off ? 'This browser cannot play the sound' : muted ? 'Unmute' : 'Mute'}
  >
    <svg viewBox="0 0 24 24" class="size-5" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M4 9.5v5h3.5L12 18.5v-13L7.5 9.5z" fill="currentColor" fill-opacity=".12" />
      {#if silent}<path d="M16.5 9.5l5 5m0-5-5 5" />{:else}<path d="M16 9a4.2 4.2 0 0 1 0 6" />{#if level > 50}<path d="M18.6 6.4a8 8 0 0 1 0 11.2" />{/if}{/if}
    </svg>
  </button>
  <input
    class="h-1.5 w-20 cursor-pointer accent-accent disabled:opacity-40"
    type="range"
    min="0"
    max="100"
    step="5"
    value={muted ? 0 : level}
    disabled={off}
    aria-label="Volume"
    oninput={(e) => slide(Number(e.currentTarget.value))}
  />
</div>
