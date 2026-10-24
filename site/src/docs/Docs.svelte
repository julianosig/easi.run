<script lang="ts">
  // The documentation: a top bar, the sections in a sidebar, and one section at a time, chosen by the address's hash
  // (#/configuration). Moving to a section titles the page, scrolls to its top and puts focus on its heading.
  import { tick, type Component } from 'svelte';
  import Wordmark from '../lib/Wordmark.svelte';
  import { REPO } from '../lib/site.ts';
  import { focus } from '../lib/focus.ts';
  import Agent from './pages/Agent.svelte';
  import Agents from './pages/Agents.svelte';
  import Api from './pages/Api.svelte';
  import Browser from './pages/Browser.svelte';
  import Clients from './pages/Clients.svelte';
  import Configuration from './pages/Configuration.svelte';
  import Demo from './pages/Demo.svelte';
  import Egress from './pages/Egress.svelte';
  import Operations from './pages/Operations.svelte';
  import Overview from './pages/Overview.svelte';
  import QuickStart from './pages/QuickStart.svelte';
  import Security from './pages/Security.svelte';
  import Sessions from './pages/Sessions.svelte';

  type Section = { id: string; title: string; page: Component };
  const groups: { name: string; sections: Section[] }[] = [
    {
      name: 'Start',
      sections: [
        { id: 'overview', title: 'Overview', page: Overview },
        { id: 'quick-start', title: 'Quick start', page: QuickStart },
        { id: 'sessions', title: 'Sessions', page: Sessions },
      ],
    },
    {
      name: 'Configure',
      sections: [
        { id: 'configuration', title: 'Configuration', page: Configuration },
        { id: 'egress', title: 'Egress proxies', page: Egress },
        { id: 'browser', title: 'The browser', page: Browser },
      ],
    },
    {
      name: 'Build on it',
      sections: [
        { id: 'agents', title: 'Agents', page: Agents },
        { id: 'agent', title: 'The agent', page: Agent },
        { id: 'api', title: 'HTTP API', page: Api },
        { id: 'clients', title: 'Custom clients', page: Clients },
      ],
    },
    {
      name: 'Run it',
      sections: [
        { id: 'operations', title: 'Operations', page: Operations },
        { id: 'demo', title: 'Public demo', page: Demo },
        { id: 'security', title: 'Security', page: Security },
      ],
    },
  ];
  const all = groups.flatMap((g) => g.sections);
  const first = all[0]!;

  const fromHash = () => (all.find((s) => `#/${s.id}` === location.hash) ?? first).id;
  let currentId = $state(fromHash());
  const current = $derived(all.find((s) => s.id === currentId) ?? first);
  let heading: HTMLElement | undefined = $state();
  let mounted = false;

  async function show() {
    currentId = fromHash();
    document.title = `${current.title} · easi.run docs`;
    if (!mounted) {
      mounted = true;
      return;
    }
    await tick();
    scrollTo({ top: 0 });
    heading?.querySelector('h1')?.focus();
  }
  show();

  const index = $derived(all.findIndex((s) => s.id === currentId));
  const previous = $derived(all[index - 1]);
  const next = $derived(all[index + 1]);
</script>

<svelte:window onhashchange={show} />

<header class="sticky top-0 z-30 border-b border-line bg-paper/90 backdrop-blur-md">
  <div class="mx-auto flex h-16 max-w-[80rem] items-center gap-3 px-5">
    <a href="/" class="flex items-center gap-2 rounded-lg {focus}" aria-label="easi.run docs home">
      <Wordmark class="text-[21px]" mark />
      <span class="ml-1 rounded-full bg-accent-soft px-2 py-0.5 text-xs font-bold uppercase tracking-wider text-accent">Docs</span>
    </a>
    <nav class="ml-auto flex items-center gap-1 text-sm font-semibold" aria-label="Site">
      <a href="https://easi.run/" class="hidden rounded-lg px-3 py-2 sm:block text-muted hover:bg-wash hover:text-ink {focus}">Home</a>
      <a href="https://easi.run/#demo" class="hidden rounded-lg px-3 py-2 sm:block text-muted hover:bg-wash hover:text-ink {focus}">Live demo</a>
      <a href={REPO} class="flex items-center gap-2 rounded-lg px-3 py-2 text-muted hover:bg-wash hover:text-ink {focus}">
        <svg viewBox="0 0 24 24" class="size-4" fill="currentColor" aria-hidden="true"><path d="M12 2a10 10 0 0 0-3.2 19.5c.5.1.7-.2.7-.5v-1.7c-2.8.6-3.4-1.3-3.4-1.3-.4-1.2-1.1-1.5-1.1-1.5-.9-.6.1-.6.1-.6 1 .1 1.5 1 1.5 1 .9 1.5 2.4 1.1 2.9.8.1-.6.4-1.1.6-1.3-2.2-.3-4.6-1.1-4.6-5 0-1.1.4-2 1-2.7-.1-.3-.4-1.3.1-2.7 0 0 .8-.3 2.8 1a9.6 9.6 0 0 1 5 0c2-1.3 2.8-1 2.8-1 .5 1.4.2 2.4.1 2.7.6.7 1 1.6 1 2.7 0 3.9-2.3 4.7-4.6 5 .4.3.7.9.7 1.9V21c0 .3.2.6.7.5A10 10 0 0 0 12 2z" /></svg>
        GitHub
      </a>
    </nav>
  </div>
</header>

<div class="mx-auto grid max-w-[80rem] gap-10 px-5 md:grid-cols-[14rem_minmax(0,1fr)]">
  <label class="mt-6 block md:hidden">
    <span class="sr-only">Section</span>
    <select
      class="w-full rounded-xl border border-line bg-paper px-4 py-3 text-[15px] font-semibold {focus}"
      value={currentId}
      onchange={(e) => (location.hash = `#/${e.currentTarget.value}`)}
    >
      {#each groups as group (group.name)}
        <optgroup label={group.name}>
          {#each group.sections as s (s.id)}<option value={s.id}>{s.title}</option>{/each}
        </optgroup>
      {/each}
    </select>
  </label>
  <nav class="hidden md:sticky md:top-16 md:block md:h-[calc(100vh-4rem)] md:overflow-y-auto md:py-8" aria-label="Documentation">
    {#each groups as group (group.name)}
      <p class="mt-6 px-3 text-xs font-bold uppercase tracking-wider text-muted first:mt-4 md:first:mt-0">{group.name}</p>
      <ul class="mt-1.5 space-y-0.5">
        {#each group.sections as s (s.id)}
          <li>
            <a
              href="#/{s.id}"
              class="block rounded-lg px-3 py-1.5 text-[15px] font-medium transition {s.id === currentId ? 'bg-accent-soft text-accent' : 'text-ink hover:bg-wash'} {focus}"
              aria-current={s.id === currentId ? 'page' : undefined}>{s.title}</a
            >
          </li>
        {/each}
      </ul>
    {/each}
  </nav>

  <main class="doc min-w-0 max-w-[48rem] pb-24 pt-6 md:pt-10" bind:this={heading}>
    {#key current}
      <current.page />
    {/key}
    <div class="mt-16 flex justify-between gap-4 border-t border-line pt-6 text-sm">
      {#if previous}<a href="#/{previous.id}" class="rounded no-underline {focus}">← {previous.title}</a>{:else}<span></span>{/if}
      {#if next}<a href="#/{next.id}" class="rounded no-underline {focus}">{next.title} →</a>{/if}
    </div>
  </main>
</div>
