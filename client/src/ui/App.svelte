<script lang="ts">
  // The admin page's views: the administrator's (create the password, log in, sessions, a session) or, in the public
  // demo, the visitor's (the door, their visit, its end).
  import Bar from './Bar.svelte';
  import Door from './Door.svelte';
  import End from './End.svelte';
  import Enroll from './Enroll.svelte';
  import Login from './Login.svelte';
  import Session from './Session.svelte';
  import Sessions from './Sessions.svelte';
  import { api, type DemoInfo, linkedSession, logout, me, messageOf, setCsrf, type Status, type Visit } from './api.ts';
  import { linkedSetup } from './setup.ts';
  import { button } from './ui.ts';

  type View =
    | { kind: 'loading' }
    | { kind: 'failed'; why: string }
    | { kind: 'enroll' }
    | { kind: 'login' }
    | { kind: 'sessions' }
    | { kind: 'session'; id: string }
    | { kind: 'door'; demo: DemoInfo }
    | { kind: 'visit'; visit: Visit; demo: DemoInfo }
    | { kind: 'end'; why: 'over' | 'lost' | 'left' | 'moved'; seconds: number };

  let view: View = $state({ kind: 'loading' });
  /**
   * The setup link opened in this tab (see `linkedSetup`), taken out of the address bar at once, so that its token stays
   * out of the history and of screenshots.
   */
  function takeSetup(): { token: string | null; cut: boolean } {
    const linked = linkedSetup(location.hash);
    if (location.hash.startsWith('#setup=')) history.replaceState(null, '', location.pathname + location.search);
    return linked;
  }
  let setup = $state(takeSetup());

  /** The administrator's view once logged in: the session a watch link names (an agent's `watch_url`), or the list. */
  const home = (): View => {
    const id = linkedSession(location.hash);
    return id ? { kind: 'session', id } : { kind: 'sessions' };
  };
  /** The session shown, which a hash change leaving it lets end a recording whole first, as its Sessions button does. */
  let shown: { leave(): Promise<void> } | undefined = $state();
  // A watch link opened in this tab while logged in, or the browser's Back and Forward between the list and one.
  $effect(() => {
    const opened = async () => {
      // A setup link opened in this tab once the page was there: the first screen takes it in place of any other.
      if (location.hash.startsWith('#setup=')) {
        setup = takeSetup();
        return;
      }
      const next = home();
      if (view.kind === 'session' && (next.kind !== 'session' || next.id !== view.id)) await shown?.leave();
      // The hash read again: it may have moved on while the recording ended.
      if (view.kind === 'sessions' || view.kind === 'session') view = home();
    };
    addEventListener('hashchange', opened);
    return () => removeEventListener('hashchange', opened);
  });
  /**
   * Leaves a session for the list, where the link that opened it no longer does, or for the login, which keeps the
   * link so that logging in again reopens the session it names.
   */
  function toSessions(to: 'login' | 'sessions'): void {
    if (to === 'sessions' && linkedSession(location.hash)) history.replaceState(null, '', location.pathname + location.search);
    view = { kind: to };
  }

  async function start() {
    try {
      const status = await api<Status>('GET', '/api/status');
      if (status.demo) {
        const demo = status.demo;
        try {
          const { csrf, demo: visitor } = await me();
          setCsrf(csrf);
          if (visitor?.visit) {
            view = { kind: 'visit', visit: visitor.visit, demo };
            return;
          }
        } catch {
          // Not logged in: the door.
        }
        view = { kind: 'door', demo };
        return;
      }
      if (!status.enrolled) {
        view = { kind: 'enroll' };
        return;
      }
      try {
        setCsrf((await me()).csrf);
        view = home();
      } catch {
        view = { kind: 'login' };
      }
    } catch (e) {
      view = { kind: 'failed', why: messageOf(e) };
    }
  }
  start();

  async function signOut() {
    await logout();
    location.reload();
  }
</script>

{#if view.kind === 'visit'}
  {@const { visit, demo } = view}
  {#key visit.session}
    <Session
      id={visit.session}
      demoLeft={visit.seconds_left}
      demoSite={visit.site}
      demoSites={demo.sites}
      demoSeconds={demo.seconds}
      demoEmbedders={demo.embedders}
      onExit={(to) => (view = { kind: 'end', why: to === 'lost' || to === 'left' || to === 'moved' ? to : 'over', seconds: demo.seconds })}
      onSwitch={(next) => (view = { kind: 'visit', visit: next, demo })}
    />
  {/key}
{:else if view.kind === 'session'}
  {@const id = view.id}
  {#key id}
    <Session bind:this={shown} {id} onExit={(to) => toSessions(to === 'login' ? 'login' : 'sessions')} />
  {/key}
{:else if view.kind === 'end'}
  <End why={view.why} seconds={view.seconds} />
{:else}
  <div class="px-4 pb-12 pt-3">
    <Bar>
      {#if view.kind === 'sessions'}<button class={button} type="button" onclick={signOut}>Log out</button>{/if}
    </Bar>
    {#if view.kind === 'loading'}
      <p class="mt-16 text-center text-muted">Loading…</p>
    {:else if view.kind === 'failed'}
      <p class="mt-16 text-center font-semibold text-bad">{view.why}</p>
    {:else if view.kind === 'enroll'}
      {#key setup}
        <Enroll token={setup.token} cut={setup.cut} onDone={() => (view = home())} onLogin={() => (view = { kind: 'login' })} />
      {/key}
    {:else if view.kind === 'login'}
      <Login onDone={() => (view = home())} />
    {:else if view.kind === 'sessions'}
      <Sessions onWatch={(id) => (view = { kind: 'session', id })} onLogin={() => (view = { kind: 'login' })} />
    {:else if view.kind === 'door'}
      {@const demo = view.demo}
      <Door {demo} onVisit={(visit) => (view = { kind: 'visit', visit, demo })} />
    {/if}
    <!-- AGPL-3.0 §13: whoever runs a modified copy offers its source to its users; this link is where they point it. -->
    <p class="mt-16 text-center text-xs text-muted">
      easi.run is free software under the AGPL-3.0 ·
      <a class="font-semibold text-accent hover:underline" href="https://github.com/julianosig/easi.run">Source</a>
    </p>
  </div>
{/if}
