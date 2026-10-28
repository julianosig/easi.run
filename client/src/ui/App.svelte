<script lang="ts">
  // The admin page's views: the administrator's (create the password, then the offer of a second step at sign-in;
  // log in; then its pages, each at its own address: see route.ts) or, in the public demo, the visitor's (the door, their visit, its end).
  import { t } from '../i18n/index.svelte.ts';
  import Bar from './Bar.svelte';
  import Door from './Door.svelte';
  import End from './End.svelte';
  import Loading from './Loading.svelte';
  import { hashOf, type Page, type Route, routeOf } from './route.ts';
  import Session from './Session.svelte';
  import { api, type DemoInfo, logout, me, messageOf, setCsrf, type Status, type Visit } from './api.ts';

  type View =
    | { kind: 'loading' }
    | { kind: 'failed'; why: string }
    | { kind: 'enroll' }
    /** The password just created, which adding a passkey asks for: kept in memory, for this screen only. */
    | { kind: 'protect'; password: string }
    | { kind: 'login'; passkey: boolean }
    | { kind: 'page'; page: Page }
    | { kind: 'session'; id: string }
    | { kind: 'door'; demo: DemoInfo }
    | { kind: 'visit'; visit: Visit; demo: DemoInfo }
    | { kind: 'end'; why: 'over' | 'lost' | 'left' | 'moved'; seconds: number };

  /** The page's language, loading: the loading state (no words but for screen readers) stays until it is in. */
  let { language }: { language: Promise<void> } = $props();
  let view: View = $state({ kind: 'loading' });
  /** The administrator's views (see views.ts), asked for once. */
  let views: ReturnType<typeof loadViews> | undefined;
  const loadViews = () => import('./views.ts');
  const adminViews = () => (views ??= loadViews());
  /** The administrator's view once logged in: the page the address names (an agent's `watch_url` names a session). */
  const home = (): View => {
    const route = routeOf(location.hash);
    return route.page === 'session' ? { kind: 'session', id: route.id } : { kind: 'page', page: route.page };
  };
  /** The session shown, which a hash change leaving it lets end a recording whole first, as its Sessions button does. */
  let shown: { leave(): Promise<void> } | undefined = $state();
  /** How many times the open page was opened again from the sidebar: each starts it afresh. */
  let reopened = $state(0);
  // A link to a page opened in this tab while logged in (the sidebar's, a watch link), or the browser's Back and Forward.
  $effect(() => {
    const opened = async () => {
      const next = home();
      if (view.kind === 'session' && (next.kind !== 'session' || next.id !== view.id)) await shown?.leave();
      // The hash read again: it may have moved on while the recording ended.
      if (view.kind === 'page' || view.kind === 'session') view = home();
    };
    addEventListener('hashchange', opened);
    return () => removeEventListener('hashchange', opened);
  });
  /**
   * Opens `route` as a new entry of the history (the browser's Back returns), or in place of the current one (a
   * closed session's, which Back must not reopen).
   */
  function go(route: Route, replace = false): void {
    const hash = hashOf(route);
    if (!replace && location.hash !== hash) {
      // The hash change shows it (see above).
      location.hash = hash;
      return;
    }
    if (location.hash !== hash) history.replaceState(null, '', hash);
    view = home();
  }
  /**
   * Leaves a session for the list, or for the login, which keeps the link so that logging in again reopens the
   * session it names.
   */
  function fromSession(to: 'login' | 'sessions' | 'closed'): void {
    if (to === 'login') toLogin();
    else go({ page: 'sessions' }, to === 'closed');
  }

  /** The login, offering a passkey when one is added (the status says so). */
  async function toLogin(): Promise<void> {
    const from = view;
    const status = await api<Status>('GET', '/api/status').catch(() => null);
    // Unless the page moved on meanwhile (a password set in this tab goes on to the second step it offers).
    if (view === from) view = { kind: 'login', passkey: status?.passkey === true };
  }

  async function start() {
    try {
      // Asked at once: who this browser is (or that it is no one yet), and what the admin is.
      const asked = me();
      asked.catch(() => undefined);
      const status = await api<Status>('GET', '/api/status');
      // The administrator's views load while the page's language may still be loading.
      if (!status.demo) void adminViews();
      await language;
      if (status.demo) {
        const demo = status.demo;
        try {
          const { csrf, demo: visitor } = await asked;
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
        setCsrf((await asked).csrf);
        view = home();
      } catch {
        view = { kind: 'login', passkey: status.passkey === true };
      }
    } catch (e) {
      await language;
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
    <Session bind:this={shown} {id} onExit={(to) => fromSession(to === 'login' || to === 'closed' ? to : 'sessions')} />
  {/key}
{:else if view.kind === 'end'}
  <End why={view.why} seconds={view.seconds} />
{:else if view.kind === 'page'}
  {@const page = view.page}
  {#await adminViews() then A}
    <A.Shell {page} onLogout={signOut} onReopen={() => reopened++}>
      {#key reopened}
      {#if page === 'sessions'}
        <A.Sessions onWatch={(id) => go({ page: 'session', id })} onLogin={toLogin} onAgents={() => go({ page: 'agents' })} onProxies={() => go({ page: 'proxies' })} />
      {:else if page === 'agents'}
        <A.Tokens onLogin={toLogin} />
      {:else if page === 'proxies'}
        <A.Proxies onLogin={toLogin} />
      {:else}
        <A.Security onLogin={toLogin} />
      {/if}
      {/key}
    </A.Shell>
  {:catch why}
    <p class="mt-16 text-center font-semibold text-bad">{messageOf(why)}</p>
  {/await}
{:else}
  <div class="px-4 pb-12 pt-3">
    <!-- The demo door's bar; the screens before a login (create the password, the second
         step it offers, log in) span the window over this padding and carry the mark themselves (see Gate). -->
    {#if view.kind === 'door'}
      <Bar />
    {/if}
    {#if view.kind === 'loading'}
      <Loading said={t('account.loading')} />
    {:else if view.kind === 'failed'}
      <p class="mt-16 text-center font-semibold text-bad">{view.why}</p>
    {:else if view.kind === 'enroll' || view.kind === 'protect' || view.kind === 'login'}
      {@const before = view}
      {#await adminViews() then A}
        {#if before.kind === 'enroll'}
          <A.Enroll onDone={(password) => (view = { kind: 'protect', password })} onLogin={toLogin} />
        {:else if before.kind === 'protect'}
          <A.Protect password={before.password} onDone={() => (view = home())} />
        {:else}
          <A.Login passkey={before.passkey} onDone={() => (view = home())} />
        {/if}
      {:catch why}
        <p class="mt-16 text-center font-semibold text-bad">{messageOf(why)}</p>
      {/await}
    {:else if view.kind === 'door'}
      {@const demo = view.demo}
      <Door {demo} onVisit={(visit) => (view = { kind: 'visit', visit, demo })} />
    {/if}
  </div>
{/if}
