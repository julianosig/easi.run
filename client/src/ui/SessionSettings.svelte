<script lang="ts">
  // The administrator's settings of one running session: how it streams to every viewer (codec, bitrate, frame-rate
  // cap), the network it simulates on their streams (for testing), and what its containers use now. A save sets the
  // stream and the network at once, for every viewer.
  import Button from './Button.svelte';
  import Dialog from './Dialog.svelte';
  import Field from './Field.svelte';
  import { ICONS } from './icons.ts';
  import { i18n, type Said, t } from '../i18n/index.svelte.ts';
  import { api, type CodecChoice, formatSize, type NetworkSim, type StreamSettings, statusOf } from './api.ts';
  import { errorOf } from './errors.ts';
  import { field } from './ui.ts';
  import { untrack } from 'svelte';

  let {
    id,
    settings,
    network,
    open = $bindable(false),
    onSaved,
    onLogin,
  }: {
    id: string;
    settings: StreamSettings;
    network: NetworkSim;
    open?: boolean;
    /** The stream or the network as the admin keeps it now, each as soon as it is saved. */
    onSaved: (saved: { settings?: StreamSettings; network?: NetworkSim }) => void;
    onLogin: () => void;
  } = $props();

  /** The admin's ranges (protocol/src/frame.rs, admin/src/api.rs). */
  const BITRATE = [100, 50_000] as const;
  const FPS = [1, 120] as const;
  const LATENCY_MAX = 2000;
  const LOSS_MAX = 50;
  const BANDWIDTH_MAX = 1_000_000;
  const CODECS: CodecChoice[] = ['auto', 'av1', 'h264', 'tiles_only'];

  let codec: CodecChoice = $state('auto');
  let bitrate = $state(2500);
  let fps = $state(60);
  let latency = $state(0);
  let loss = $state(0);
  let bandwidth = $state(0);
  let saving = $state(false);
  let error: Said | null = $state(null);
  let saved = $state(false);

  /** What each container uses now (`GET /api/sessions/{id}/stats`), and how many viewers watch. */
  interface Container {
    status?: string;
    running?: boolean;
    cpu_percent?: number | null;
    memory_bytes?: number | null;
    memory_limit_bytes?: number | null;
    error?: string;
  }
  let usage: { containers: Record<string, Container>; viewers: number } | null = $state(null);
  let usageError: Said | null = $state(null);

  /** Whether the person changed the stream's or the network's fields since the dialog opened: the session's own
   * values then no longer replace those. */
  let streamEdited = false;
  let networkEdited = false;
  /** The latest usage read: an earlier one's late answer (from before a save or a reopening) changes nothing. */
  let usageTurn = 0;
  async function readUsage() {
    const mine = ++usageTurn;
    usageError = null;
    try {
      const got = await api<{ session: { settings: StreamSettings; network: NetworkSim } | null; containers: { containers?: Record<string, Container>; error?: string }; viewers: unknown[] }>('GET', `/api/sessions/${id}/stats`);
      if (mine !== usageTurn || !open) return;
      // The session as the admin has it now (another tab may have changed it), in the fields the person has not edited.
      if (got.session) {
        if (!streamEdited) ({ codec, bitrate_kbps: bitrate, max_fps: fps } = got.session.settings);
        if (!networkEdited) ({ latency_ms: latency, loss_percent: loss, bandwidth_kbps: bandwidth } = got.session.network);
        onSaved({ settings: got.session.settings, network: got.session.network });
      }
      usage = { containers: got.containers.containers ?? {}, viewers: got.viewers.length };
      if (got.containers.error) usageError = () => got.containers.error ?? '';
    } catch (e) {
      if (mine !== usageTurn || !open) return;
      if (statusOf(e) === 401) return onLogin();
      usageError = errorOf(e);
    }
  }

  // Opened, it starts from what the session has now; a save changing that meanwhile does not start it over.
  $effect(() => {
    if (!open) return;
    untrack(() => {
      ({ codec, bitrate_kbps: bitrate, max_fps: fps } = settings);
      ({ latency_ms: latency, loss_percent: loss, bandwidth_kbps: bandwidth } = network);
      error = null;
      saved = false;
      streamEdited = networkEdited = false;
      usage = null;
      void readUsage();
    });
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    saving = true;
    // A usage read out now answers with what the session had before this save.
    usageTurn++;
    error = null;
    saved = false;
    const stream: StreamSettings = { codec, bitrate_kbps: bitrate, max_fps: fps };
    const net: NetworkSim = { latency_ms: latency, loss_percent: loss, bandwidth_kbps: bandwidth };
    try {
      await api('PUT', `/api/sessions/${id}/settings`, stream);
      onSaved({ settings: stream });
      await api('PUT', `/api/sessions/${id}/network`, net);
      onSaved({ network: net });
      saved = true;
    } catch (e) {
      if (statusOf(e) === 401) return onLogin();
      error = errorOf(e);
    }
    saving = false;
  }

  const number = (v: number, digits = 0) => v.toLocaleString(i18n.locale, { maximumFractionDigits: digits });
</script>

<Dialog title={t('session.settings.title')} icon={ICONS.edit} subtitle={t('session.settings.subtitle')} bind:open>
  <form onsubmit={save} oninput={() => (saved = false)}>
    <!-- Disabled while a save is out, so what it says it saved is what the fields show. -->
    <fieldset class="m-0 min-w-0 border-0 p-0" disabled={saving}>
    <div oninput={() => (streamEdited = true)}>
    <p class="mt-6 text-sm font-semibold">{t('session.settings.stream')}</p>
    <Field icon={ICONS.video} label={t('session.settings.codec')} for="settings-codec" help={t('session.settings.codec.help')}>
      <select id="settings-codec" class={field} aria-describedby="settings-codec-help" bind:value={codec}>
        {#each CODECS as c (c)}<option value={c}>{t(`session.settings.codec.${c}`)}</option>{/each}
      </select>
    </Field>
    <Field icon={ICONS.bitrate} label={t('session.settings.bitrate')} for="settings-bitrate" help={t('session.settings.bitrate.help', { min: BITRATE[0], max: BITRATE[1] })}>
      <input id="settings-bitrate" class={field} type="number" inputmode="numeric" min={BITRATE[0]} max={BITRATE[1]} step="1" required aria-describedby="settings-bitrate-help" bind:value={bitrate} />
    </Field>
    <Field icon={ICONS.frames} label={t('session.settings.fps')} for="settings-fps" help={t('session.settings.fps.help', { min: FPS[0], max: FPS[1] })}>
      <input id="settings-fps" class={field} type="number" inputmode="numeric" min={FPS[0]} max={FPS[1]} step="1" required aria-describedby="settings-fps-help" bind:value={fps} />
    </Field>

    </div>
    <div oninput={() => (networkEdited = true)}>
    <p class="mt-8 text-sm font-semibold">{t('session.settings.network')}</p>
    <p class="text-[13px] text-muted">{t('session.settings.network.help')}</p>
    <Field icon={ICONS.network} label={t('session.settings.latency')} for="settings-latency" help={t('session.settings.latency.help', { max: LATENCY_MAX })}>
      <input id="settings-latency" class={field} type="number" inputmode="numeric" min="0" max={LATENCY_MAX} step="1" required aria-describedby="settings-latency-help" bind:value={latency} />
    </Field>
    <Field icon={ICONS.network} label={t('session.settings.loss')} for="settings-loss" help={t('session.settings.loss.help', { max: LOSS_MAX })}>
      <input id="settings-loss" class={field} type="number" inputmode="decimal" min="0" max={LOSS_MAX} step="any" required aria-describedby="settings-loss-help" bind:value={loss} />
    </Field>
    <Field icon={ICONS.network} label={t('session.settings.bandwidth')} for="settings-bandwidth" help={t('session.settings.bandwidth.help', { max: BANDWIDTH_MAX })}>
      <input id="settings-bandwidth" class={field} type="number" inputmode="numeric" min="0" max={BANDWIDTH_MAX} step="1" required aria-describedby="settings-bandwidth-help" bind:value={bandwidth} />
    </Field>

    </div>
    </fieldset>
    <p class="mt-4 text-sm font-semibold text-bad empty:hidden" role="alert">{error?.()}</p>
    <p class="mt-4 text-sm font-semibold text-good empty:hidden" role="status">{saved ? t('session.settings.saved') : ''}</p>
    <div class="mt-6 flex justify-end gap-3">
      <Button disabled={saving} onclick={() => (open = false)}>{t('session.settings.close')}</Button>
      <Button variant="primary" icon={ICONS.check} type="submit" disabled={saving}>{t('session.settings.save')}</Button>
    </div>
  </form>

  <section class="mt-8 border-t border-line pt-6" aria-labelledby="settings-usage">
    <div class="flex items-center justify-between gap-3">
      <h3 id="settings-usage" class="text-sm font-semibold">{t('session.settings.usage')}</h3>
      <Button size="sm" variant="ghost" icon={ICONS.retry} onclick={readUsage}>{t('session.settings.usage.refresh')}</Button>
    </div>
    <p class="mt-2 text-sm font-semibold text-bad empty:hidden" role="alert">{usageError?.()}</p>
    {#if usage}
      <p class="mt-2 text-sm text-muted">{t('session.settings.viewers', { count: usage.viewers })}</p>
      <dl class="mt-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-sm">
        {#each Object.entries(usage.containers) as [role, c] (role)}
          <dt class="font-semibold">{role}</dt>
          <dd class="m-0 text-right tabular-nums text-muted">
            {#if c.error}{c.error}
            {:else if !c.running}{t('session.settings.stopped', { status: c.status ?? '' })}
            {:else}{t('session.settings.cpu', { percent: c.cpu_percent == null ? '—' : number(c.cpu_percent, 1) })} · {c.memory_bytes == null
                ? '—'
                : c.memory_limit_bytes
                  ? t('session.settings.memory', { used: formatSize(c.memory_bytes, i18n.locale), limit: formatSize(c.memory_limit_bytes, i18n.locale) })
                  : formatSize(c.memory_bytes, i18n.locale)}{/if}
          </dd>
        {/each}
      </dl>
    {/if}
  </section>
</Dialog>
