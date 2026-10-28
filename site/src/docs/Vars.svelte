<script lang="ts">
  // A table of settings: name, default, accepted values, and what each does (config.<group>.<name>.what and .values).
  import type { Group, Var } from './config.ts';
  import { say, t } from './i18n.ts';
  let { vars, group }: { vars: Var[]; group: Group } = $props();
</script>

<table>
  <thead><tr><th>{t('config.name')}</th><th>{t('config.default')}</th><th>{t('config.what')}</th></tr></thead>
  <tbody>
    {#each vars as v (v.name)}
      <tr>
        <td><code>{v.name}</code></td>
        <td>{#if v.default === null}<span class="text-muted">{t('config.required')}</span>{:else if v.default === ''}<span class="text-muted">{t('config.unset')}</span>{:else}<code>{v.default}</code>{/if}</td>
        <td>{say(`config.${group}.${v.name}.what`)}{#if v.values}<span class="mt-1 block text-[13px] text-muted">{say(`config.${group}.${v.name}.values`)}</span>{/if}</td>
      </tr>
    {/each}
  </tbody>
</table>
