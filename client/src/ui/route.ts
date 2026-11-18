// The logged-in admin's pages, each at its own fragment of the address, so a link opens it and the browser's Back and
// Forward move between them: `#sessions` (also an empty or unknown fragment), `#replays`, `#agents`, `#settings`, `#proxies`, `#security`, and
// one session at its watch link (`#session=<id>`, see `linkedSession`).

import { linkedSession } from './api.ts';

export const PAGES = ['sessions', 'replays', 'agents', 'settings', 'proxies', 'security'] as const;
export type Page = (typeof PAGES)[number];
export type Route = { page: Page } | { page: 'session'; id: string };

/** The page a fragment names; the sessions for any fragment that names none. */
export function routeOf(hash: string): Route {
  const id = linkedSession(hash);
  if (id) return { page: 'session', id };
  const name = hash.slice(1);
  return { page: (PAGES as readonly string[]).includes(name) ? (name as Page) : 'sessions' };
}

// Whether the next session opened is opened from a session's sidebar (a plain click on one of its links), which the
// page that shows sessions asks once as it opens one: only then does a session start with its bar away (see `Dash`).
// A link, the browser's Back, or a session that started again under another id, open one as ever.
let sidebar = false;
/** A session's sidebar link was chosen, to open in this tab. */
export const fromSidebar = (): void => {
  sidebar = true;
};
/** Whether a sidebar link was chosen since this was last asked. */
export function tookSidebar(): boolean {
  const took = sidebar;
  sidebar = false;
  return took;
}

/** The fragment that opens `route`. */
export function hashOf(route: Route): string {
  return route.page === 'session' ? `#session=${route.id}` : `#${route.page}`;
}
