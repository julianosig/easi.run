<script lang="ts">
  // A proxy's check as a chip: checking, not run (the server checks others now), passed (how fast, and where its exit
  // is when found), or why it failed. Each page words checking and passing its own way.
  import { i18n, t } from '../i18n/index.svelte.ts';
  import { CHIP_TONES, chip } from './admin.ts';
  import { BUSY, type Check, checkKey, exitText } from './proxies.ts';

  let {
    state,
    checking,
    passed,
  }: { state: Check | 'checking' | 'busy'; checking: string; passed: (latency: number, exit: string | null) => string } = $props();
</script>

{#if state === 'checking'}
  <span class="{chip} {CHIP_TONES.neutral}"><span class="size-2.5 animate-spin rounded-full border-2 border-muted border-t-transparent" aria-hidden="true"></span>{checking}</span>
{:else if state === 'busy'}
  <span class="{chip} {CHIP_TONES.neutral}">{t(BUSY)}</span>
{:else if state.ok}
  <span class="{chip} {CHIP_TONES.good}"><span class="size-1.5 rounded-full bg-good" aria-hidden="true"></span>{passed(state.latency_ms, state.exit ? exitText(state.exit, i18n.locale) : null)}</span>
{:else}
  <span class="{chip} {CHIP_TONES.bad}"><span class="size-1.5 rounded-full bg-bad" aria-hidden="true"></span>{t(checkKey(state.code))}</span>
{/if}
