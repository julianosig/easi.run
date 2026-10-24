<script lang="ts">
  // A table of settings: name, default, accepted values, and what each does.
  import type { Var } from './config.ts';
  let { vars }: { vars: Var[] } = $props();
</script>

<table>
  <thead><tr><th>Name</th><th>Default</th><th>What it does</th></tr></thead>
  <tbody>
    {#each vars as v (v.name)}
      <tr>
        <td><code>{v.name}</code></td>
        <td>{#if v.default === null}<span class="text-muted">required</span>{:else if v.default === ''}<span class="text-muted">unset</span>{:else}<code>{v.default}</code>{/if}</td>
        <td>{v.what}{#if v.values}<span class="mt-1 block text-[13px] text-muted">{v.values}</span>{/if}</td>
      </tr>
    {/each}
  </tbody>
</table>
