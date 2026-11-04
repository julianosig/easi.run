<script lang="ts" module>
  /** The permissions' saves, one after another across every time this page opens in a tab: a page left with a save still
   * queued never overwrites the next page's. */
  let saves = Promise.resolve();
</script>

<script lang="ts">
  // What every agent may do with files while it holds a session's control; a change applies at once. Part of Settings.
  import { ICONS } from './icons.ts';
  import { type Said, t } from '../i18n/index.svelte.ts';
  import { row, rowBadge, rows } from './admin.ts';
  import { type AgentFiles, api, type FileChoice } from './api.ts';
  import { requests } from './requests.svelte.ts';
  import Icon from './Icon.svelte';
  import { focusRing } from './ui.ts';
  import Skeleton from './Skeleton.svelte';

  let { onLogin }: { onLogin: () => void } = $props();
  const page = requests(() => onLogin());

  /** The permissions as the page shows them, and as the admin last confirmed them. */
  let files: AgentFiles | null = $state.raw(null);
  let confirmed: AgentFiles | null = null;
  let error: Said | null = $state(null);
  // Read in the same queue as the saves, so a save an earlier page left under way lands first.
  $effect(() => {
    saves = saves.then(async () => {
      try {
        const got = await api<AgentFiles>('GET', '/api/agent-files');
        if (page.alive()) files = confirmed = got;
      } catch (e) {
        page.fail(e, (said) => (error = said));
      }
    });
  });
  /** Saves are made one after another, so the last change is what the admin keeps; each saves the page's permissions
   * as they are then, and one a later change overtook is skipped. A refused save puts back what the admin has. */
  function set(change: Partial<AgentFiles>) {
    if (!files) return;
    const wanted = (files = { ...files, ...change });
    error = null;
    saves = saves.then(async () => {
      // Gone, or overtaken by a later change (which saves this one's too): nothing to send.
      if (!page.alive() || files !== wanted) return;
      try {
        const saved = await api<AgentFiles>('PUT', '/api/agent-files', wanted);
        confirmed = saved;
        if (page.alive() && files === wanted) files = saved;
      } catch (e) {
        // A later change, still to be saved, says how it went instead.
        if (files !== wanted) return;
        files = confirmed;
        page.fail(e, (said) => (error = said));
      }
    });
  }
  const CHOICES: FileChoice[] = ['ask', 'allow', 'deny'];
  const KINDS = [
    { key: 'downloads', icon: ICONS.download },
    { key: 'transfers', icon: ICONS.send },
  ] as const;
</script>

<section class="mt-12" aria-labelledby="settings-files">
  <h2 id="settings-files" class="text-lg font-extrabold tracking-[-0.02em]">{t('agents.files.title')}</h2>
  <p class="mt-0.5 text-sm text-muted">{t('agents.files.subtitle')}</p>
  <p class="mt-3 font-semibold text-bad empty:hidden" role="alert">{error?.()}</p>

{#if files}
  <ul class="mt-3 {rows}">
    {#each KINDS as { key, icon } (key)}
      {@const id = `files-${key}`}
      <li class="flex flex-wrap items-center gap-4 py-3 sm:flex-nowrap {row}">
        <span class={rowBadge}><Icon d={icon} class="size-5" /></span>
        <span class="min-w-0 flex-1">
          <label class="block font-semibold" for={id}>{t(`agents.files.${key}`)}</label>
          <span id="{id}-help" class="mt-1 block text-sm text-muted">{t(`agents.files.${key}.help`)}</span>
        </span>
        <span class="relative shrink-0">
          <select {id} class="choice cursor-pointer rounded-lg bg-wash py-2 pl-3 pr-8 text-sm font-semibold text-ink transition hover:bg-accent-soft {focusRing}" aria-describedby="{id}-help" value={files[key]} onchange={(e) => set({ [key]: e.currentTarget.value as FileChoice })}>
            {#each CHOICES as c (c)}<option value={c}>{t(`agents.files.choice.${c}`)}</option>{/each}
          </select>
          <Icon d={ICONS.collapse} class="pointer-events-none absolute right-2.5 top-1/2 size-4 -translate-y-1/2 text-muted" />
        </span>
      </li>
    {/each}
  </ul>
{:else if !error}
  <Skeleton said={t('agents.files.loading')} />
{/if}
</section>
