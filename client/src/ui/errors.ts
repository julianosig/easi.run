// What a refusal or failure says to the person, in the page's language, by the code the admin sent (see
// admin/src/coded.rs and `toldBy`): never the admin's English, which stays for agents and logs.

import { t, type Said } from '../i18n/index.svelte.ts';
import { bodyOf, statusOf, toldBy } from './api.ts';

/** A message by a key made at run time, with its values (`t` checks the keys it knows when the page is built); the key
 * itself when there is no such message. */
const tell = t as unknown as (key: string, params?: Record<string, string | number>) => string;

/** What `coded` says (see `toldBy`), as the page's language is each time it shows. */
export function saidOf(coded: unknown, status?: number, unreached = false): Said {
  const { key, params, fallback } = toldBy(coded, status, unreached);
  return () => {
    const said = key === null ? key : tell(key, params);
    return said !== null && said !== key ? said : tell(fallback);
  };
}

/** What a failed request says: by the code its answer names; else, by its status, a refusal or a failure; or, when it
 * got no answer (no status), that the server could not be reached. */
export const errorOf = (e: unknown): Said => saidOf(bodyOf(e), statusOf(e), statusOf(e) === undefined);
