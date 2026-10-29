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
  /** Asks for the page's list again: its answer, or its failure, counts only while no later asking started (an older
   * answer arriving last would bring back what was removed meanwhile); undefined when one did. */
  latest<T>(ask: () => Promise<T>): Promise<T | undefined>;
}

/** A page's requests, made as the page is (it ends with the page). */
export function requests(onLogin: () => void): Requests {
  let alive = true;
  $effect(() => () => {
    alive = false;
  });
  let turn = 0;
  return {
    alive: () => alive,
    async latest(ask) {
      const mine = ++turn;
      try {
        const answer = await ask();
        return mine === turn ? answer : undefined;
      } catch (e) {
        if (mine === turn) throw e;
        return undefined;
      }
    },
    fail(e, say) {
      if (!alive) return;
      if (statusOf(e) === 401) return onLogin();
      say(errorOf(e));
    },
  };
}
