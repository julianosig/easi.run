// What an admin page does with the answers to its requests: once the page is gone, nothing; a lapsed login goes to the
// login; any other failure is said where the page shows it.

import type { Said } from '../i18n/index.svelte.ts';
import { statusOf } from './api.ts';
import { errorOf } from './errors.ts';

export interface Requests {
  /** Whether the page is still there for an answer that arrives now. */
  alive(): boolean;
  /** Answers a failed request: nothing once the page is gone, the login when it lapsed, else `say` with what it says
   * (see `errorOf`). */
  fail(e: unknown, say: (said: Said) => void): void;
}

/** A page's requests, made as the page is (it ends with the page). */
export function requests(onLogin: () => void): Requests {
  let alive = true;
  $effect(() => () => {
    alive = false;
  });
  return {
    alive: () => alive,
    fail(e, say) {
      if (!alive) return;
      if (statusOf(e) === 401) return onLogin();
      say(errorOf(e));
    },
  };
}
