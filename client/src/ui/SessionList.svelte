<script lang="ts">
  // A session's sidebar, for moving between sessions: the running ones (the open one marked, each opening in its place),
  // then the saved ones not running, each opening the dialog that resumes it (or, while they are locked, the sessions page, which unlocks them); New session, above them, starts a new one. Expanded it takes
  // some 15% of the window, collapsed a rail of their initials, each one's dot its state; either way the screen fits beside
  // it, never under it. Whether it is collapsed is this browser's convenience, for every session here.
  import { api, remember, remembered, type SessionInfo, type TokenInfo } from './api.ts';
  import { followLists } from './updates.ts';
  import { type Said, t } from '../i18n/index.svelte.ts';
  import Icon from './Icon.svelte';
  import Mark from './Mark.svelte';
  import { ICONS } from './icons.ts';
  import { hashOf } from './route.ts';
  import { focusRing } from './ui.ts';
  import type { ProfileInfo, ProfileList } from './profiles.ts';
  import type { Proxy, ProxyList } from './proxies.ts';
  import { requests } from './requests.svelte.ts';
  import StartSession from './StartSession.svelte';

  let {
    id,
    side = $bindable(0),
    onLogin,
  }: {
    id: string;
    /** How much of the window's left it takes from the screen, in pixels. */
    side?: number;
    onLogin: () => void;
  } = $props();

  const KEY = 'session-list';
  const RAIL = 48;

  let expanded = $state(remembered(KEY) !== 'collapsed');
  let width = $state(0);
  let sessions: SessionInfo[] = $state([]);
  let profiles: ProfileList | null = $state(null);
  let tokens: TokenInfo[] | null = $state(null);
  let egresses: Proxy[] = $state([]);
  /** Why the list could not be read (cleared once it is), and why a saved session made here did not start. */
  let error: Said | null = $state(null);
  let unstarted: Said | null = $state(null);
  let starter: StartSession | undefined = $state();

  $effect(() => {
    side = expanded ? width : RAIL;
  });

  const page = requests(() => onLogin());
  const failed = (e: unknown) => page.fail(e, (said) => (error = said));
  /** As the sessions page asks for them: what runs, the saved sessions, and what their dialog needs. */
  async function load(): Promise<void> {
    try {
      const answers = await page.latest(() =>
        Promise.all([
          api<{ sessions: SessionInfo[] }>('GET', '/api/sessions'),
          api<ProfileList>('GET', '/api/profiles'),
          api<{ tokens: TokenInfo[] }>('GET', '/api/tokens'),
          api<ProxyList>('GET', '/api/egresses'),
        ]),
      );
      if (!answers) return;
      const [listed, saved, agents, proxies] = answers;
      [sessions, profiles, tokens, egresses] = [listed.sessions, saved, agents.tokens, proxies.egresses];
      error = null;
    } catch (e) {
      failed(e);
    }
  }
  $effect(() => followLists(() => void load()));
  $effect(() => {
    load();
  });
  /** The saved sessions not running now. */
  const stopped = $derived.by((): ProfileInfo[] => (profiles as ProfileList | null)?.profiles.filter((p) => p.session === null) ?? []);

  /** The start dialog, for a new session or `resume`: what an earlier one said is behind. */
  function open(resume: ProfileInfo | null): void {
    unstarted = null;
    starter?.open(resume);
  }

  function toggle(): void {
    expanded = !expanded;
    remember(KEY, expanded ? null : 'collapsed');
  }
  const go = (route: Parameters<typeof hashOf>[0]) => (location.hash = hashOf(route));
  const hostOf = (url: string): string => url.replace(/^https?:\/\/(www\.)?/, '').replace(/[/?#].*$/, '');
  const nameOf = (s: SessionInfo): string => s.profile?.name ?? hostOf(s.url);
  /** Two letters that tell a session apart at a glance: a name's first two words', or its first two letters; a site's
   *  own name (what comes before its top-level domain) stands for its address. */
  function initials(name: string): string {
    const labels = name.split('.');
    const own = labels.length > 1 && !name.includes(' ') ? labels[labels.length - 2]! : name;
    const words = own.split(/[^\p{L}\p{N}]+/u).filter(Boolean);
    return (words.length > 1 ? words[0]![0]! + words[1]![0]! : [...(words[0] ?? '?')].slice(0, 2).join('')).toUpperCase();
  }
  const item = $derived(`flex h-9 w-full items-center gap-2.5 rounded-lg text-sm transition ${expanded ? 'px-2.5' : 'justify-center'} ${focusRing}`);
</script>

{#snippet entry(name: string, dot: string, state: string, current = false)}
  <span
    class="relative grid size-6 shrink-0 place-items-center rounded-md text-[10px] font-bold uppercase tracking-wide {current ? 'bg-accent text-white' : 'bg-line text-ink'}"
    aria-hidden="true"
  >
    {initials(name)}
    <span class="absolute -right-1 -top-1 size-2 rounded-full ring-2 ring-paper {dot}"></span>
  </span>
  <span class={expanded ? 'min-w-0 flex-1 truncate text-left' : 'sr-only'}>{name}</span>
  {#if state}<span class="sr-only">{state}</span>{/if}
{/snippet}

{#snippet add()}
  <button
    type="button"
    class="flex h-8 w-full items-center justify-center gap-1.5 rounded-lg bg-accent-soft text-sm font-semibold text-[light-dark(var(--color-accent),#fff)] transition hover:bg-accent hover:text-white disabled:opacity-50 {focusRing}"
    aria-label={t('sessions.new')}
    title={t('sessions.new')}
    disabled={profiles === null}
    onclick={() => open(null)}
  >
    <Icon d={ICONS.add} class="size-4" stroke={2.4} />{#if expanded}{t('sessions.new')}{/if}
  </button>
{/snippet}

<aside
  class="fixed inset-y-0 left-0 z-30 flex flex-col border-r border-line bg-paper {expanded ? 'w-[15vw] min-w-52 max-w-[16rem]' : 'w-12'}"
  bind:clientWidth={width}
  aria-label={t('session.list.label')}
>
  <!-- The mark, faint behind the list. -->
  {#if expanded}<Mark class="pointer-events-none absolute left-1/2 top-1/2 h-28 w-auto -translate-x-1/2 -translate-y-1/2 opacity-30" />{/if}
  <div class="relative flex h-12 shrink-0 items-center {expanded ? 'pl-4' : 'justify-center'}">
    {#if expanded}
      <a class="flex min-w-0 max-w-[calc(100%-5rem)] items-center gap-2 rounded-lg text-lg font-semibold tracking-[-0.015em] text-ink transition hover:text-accent {focusRing}" href={hashOf({ page: 'sessions' })}
        ><span class="truncate">{t('sessions.title')}</span></a
      >
    {/if}
    <button
      type="button"
      class="grid size-8 shrink-0 place-items-center rounded-lg text-muted transition hover:bg-wash hover:text-ink {expanded ? 'absolute right-2' : ''} {focusRing}"
      aria-expanded={expanded}
      aria-label={expanded ? t('session.list.hide') : t('session.list.show')}
      title={expanded ? t('session.list.hide') : t('session.list.show')}
      onclick={toggle}
    >
      <Icon d={expanded ? 'm15 6-6 6 6 6' : 'm9 6 6 6-6 6'} class="size-4" stroke={2.2} />
    </button>
  </div>
  <div class="flex min-h-0 flex-1 flex-col overflow-y-auto px-2 pb-3 {expanded ? '' : 'items-center'}">
    <div class="mb-5 {expanded ? '' : 'self-stretch'}">{@render add()}</div>
    <ul class="w-full space-y-0.5">
      {#each sessions as s (s.id)}
        {@const current = s.id === id}
        <li>
          <a
            class="{item} {current ? `font-semibold text-ink ${expanded ? 'bg-accent-soft' : ''}` : 'text-muted hover:bg-wash hover:text-ink'}"
            href={hashOf({ page: 'session', id: s.id })}
            aria-current={current ? 'page' : undefined}
            title={nameOf(s)}
          >
            {@render entry(nameOf(s), s.failed ? 'bg-bad' : s.help ? 'bg-you' : 'bg-good', s.failed ? t('sessions.status.failed') : s.help ? t('sessions.status.needsYou') : '', current)}
          </a>
        </li>
      {/each}
      {#each stopped as p (p.id)}
        <li>
          <button type="button" class="{item} text-muted hover:bg-wash hover:text-ink" title={p.name} onclick={() => (profiles?.unlocked ? open(p) : go({ page: 'sessions' }))}>
            {@render entry(p.name, 'bg-line', t('session.list.stopped'))}
          </button>
        </li>
      {/each}
    </ul>
    {#if (unstarted ?? error) && expanded}<p class="mt-2 px-2.5 text-xs font-semibold text-bad" role="alert">{(unstarted ?? error)?.()}</p>{/if}
  </div>
</aside>

<StartSession
  bind:this={starter}
  bind:profiles
  {egresses}
  {tokens}
  alive={page.alive}
  {failed}
  onStarted={(started) => go({ page: 'session', id: started })}
  onUnstarted={(said) => (unstarted = said)}
  onChange={load}
  onAgents={() => go({ page: 'agents' })}
  onProxies={() => go({ page: 'proxies' })}
/>
