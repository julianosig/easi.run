<script lang="ts">
  // The administrator's scripted agent: one tool call at a time, its result shown as text.
  import { api, messageOf } from './api.ts';
  import { button, field, label, panelTitle } from './ui.ts';

  let { id }: { id: string } = $props();
  let tool = $state('list_windows');
  let args = $state('{}');
  let out = $state('');

  async function run() {
    out = '…';
    try {
      out = JSON.stringify(await api('POST', `/api/sessions/${id}/agent`, { tool, arguments: JSON.parse(args || '{}') }), null, 2);
    } catch (e) {
      out = messageOf(e);
    }
  }
</script>

<h2 class={panelTitle}>Agent</h2>
<label class={label} for="tool">Tool</label>
<input id="tool" class={field} type="text" bind:value={tool} />
<label class={label} for="args">Arguments (JSON)</label>
<textarea id="args" class="{field} min-h-20 font-mono text-sm" bind:value={args}></textarea>
<button class="{button} mt-4" type="button" onclick={run}>Run</button>
{#if out}<pre class="mt-3 max-h-80 overflow-auto whitespace-pre-wrap break-words text-xs text-muted">{out}</pre>{/if}
