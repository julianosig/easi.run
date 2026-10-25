// The logged-in admin's pages, each at its own fragment of the address, so a link opens it and the browser's Back and
// Forward move between them: `#sessions` (also an empty or unknown fragment), `#agents`, `#proxies`, `#security`, and
// one session at its watch link (`#session=<id>`, see `linkedSession`).

import { linkedSession } from './api.ts';

export const PAGES = ['sessions', 'agents', 'proxies', 'security'] as const;
export type Page = (typeof PAGES)[number];
export type Route = { page: Page } | { page: 'session'; id: string };

/** The page a fragment names; the sessions for any fragment that names none. */
export function routeOf(hash: string): Route {
  const id = linkedSession(hash);
  if (id) return { page: 'session', id };
  const name = hash.slice(1);
  return { page: (PAGES as readonly string[]).includes(name) ? (name as Page) : 'sessions' };
}

/** The fragment that opens `route`. */
export function hashOf(route: Route): string {
  return route.page === 'session' ? `#session=${route.id}` : `#${route.page}`;
}
