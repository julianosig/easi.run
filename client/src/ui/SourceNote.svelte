<script lang="ts">
  // AGPL-3.0 §13: whoever runs a modified copy offers its source to its users; this link is where they point it.
  import { t } from '../i18n/index.svelte.ts';

  let { class: className = '' }: { class?: string } = $props();
</script>

<p class="text-center text-xs text-muted {className}">
  {t('common.license')}
  <a class="font-semibold text-accent hover:underline" href="https://github.com/julianosig/easi.run">{t('common.source')}</a>
</p>
