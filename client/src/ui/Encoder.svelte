<script lang="ts">
  // The administrator's encoder and network switches for one session, starting from what the session has now.
  import { untrack } from 'svelte';
  import { api, type EncoderSettings, type NetworkConditions } from './api.ts';
  import { button, field, label, panelTitle } from './ui.ts';

  let { id, fail, settings, network }: { id: string; fail: (e: unknown) => void; settings: EncoderSettings; network: NetworkConditions } =
    $props();
  let codec = $state(untrack(() => settings.codec));
  let bitrate = $state(untrack(() => settings.bitrate_kbps));
  let fps = $state(untrack(() => settings.max_fps));
  let latency = $state(untrack(() => network.latency_ms));
  let loss = $state(untrack(() => network.loss_percent));
  let bandwidth = $state(untrack(() => network.bandwidth_kbps));

  const apply = () => api('PUT', `/api/sessions/${id}/settings`, { codec, bitrate_kbps: bitrate, max_fps: fps }).catch(fail);
  const simulate = () =>
    api('PUT', `/api/sessions/${id}/network`, { latency_ms: latency, loss_percent: loss, bandwidth_kbps: bandwidth }).catch(fail);
</script>

<h2 class={panelTitle}>Encoder</h2>
<label class={label} for="codec">Codec</label>
<select id="codec" class={field} bind:value={codec}>{#each ['auto', 'av1', 'h264', 'tiles_only'] as c (c)}<option value={c}>{c}</option>{/each}</select>
<label class={label} for="bitrate">Bitrate (kbit/s)</label>
<input id="bitrate" class={field} type="number" min="100" max="50000" bind:value={bitrate} />
<label class={label} for="fps">Frame-rate cap</label>
<input id="fps" class={field} type="number" min="1" max="120" bind:value={fps} />
<button class="{button} mt-4" type="button" onclick={apply}>Apply</button>

<h2 class="{panelTitle} mt-6">Network</h2>
<label class={label} for="latency">Latency (ms)</label>
<input id="latency" class={field} type="number" min="0" max="2000" bind:value={latency} />
<label class={label} for="loss">Loss (%)</label>
<input id="loss" class={field} type="number" min="0" max="50" step="0.5" bind:value={loss} />
<label class={label} for="bandwidth">Bandwidth (kbit/s, 0 = unlimited)</label>
<input id="bandwidth" class={field} type="number" min="0" max="1000000" bind:value={bandwidth} />
<button class="{button} mt-4" type="button" onclick={simulate}>Simulate</button>
