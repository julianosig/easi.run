<script lang="ts">
  // The admin's settings for every agent: what they may do with files, and where they call you from.
  import AgentFiles from './AgentFiles.svelte';
  import ReplayQuality from './ReplayQuality.svelte';
  import { ICONS } from './icons.ts';
  import { t } from '../i18n/index.svelte.ts';
  import Notifications from './Notifications.svelte';
  import PageHeader from './PageHeader.svelte';

  let { onLogin }: { onLogin: () => void } = $props();
</script>

<PageHeader icon={ICONS.settings} title={t('shell.page.settings')} subtitle={t('agents.settings.subtitle')} />
<AgentFiles {onLogin} />
<ReplayQuality {onLogin} />
<Notifications {onLogin} />
