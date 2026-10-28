// The admin API, as admin/src/api.rs serves it: same-origin cookies, and the session's CSRF token on every
// state-changing request. Errors carry the HTTP status and the admin's body, whose code (admin/src/coded.rs) a view
// tells a lapsed login (401), a busy demo (409) or a failed check by, and `errorOf` says in the page's language.

import type { Features } from '../viewer.ts';

/** Marks the next demo start as the page's own (after time ran out), for the page after its reload. */
export const AUTO_START = 'easi-run:auto-start';

let csrf = '';

export function setCsrf(token: string): void {
  csrf = token;
}

type ApiError = Error & { status?: number; body?: Record<string, unknown> };

export async function api<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = {};
  if (body !== undefined) headers['content-type'] = 'application/json';
  if (method !== 'GET') headers['x-csrf-token'] = csrf;
  const response = await fetch(path, {
    method,
    headers,
    body: body === undefined ? null : JSON.stringify(body),
    credentials: 'same-origin',
  });
  const data: unknown = await response.json().catch(() => ({}));
  if (!response.ok) {
    // The admin's English stays in the body, for agents and logs; the page says the code (see `errorOf`).
    throw Object.assign(new Error(`${method} ${path}: ${response.status}`), {
      status: response.status,
      body: (data !== null && typeof data === 'object' ? data : {}) as Record<string, unknown>,
    });
  }
  return data as T;
}

export const statusOf = (e: unknown): number | undefined => (e as ApiError).status;
export const bodyOf = (e: unknown): Record<string, unknown> => (e as ApiError).body ?? {};
/** The code a refusal or failure names (admin/src/coded.rs), if it names one. */
export const codeOf = (e: unknown): string | undefined => {
  const code = bodyOf(e).code;
  return typeof code === 'string' ? code : undefined;
};

/** What a refusal or failure says, before it is translated (see errors.ts): the message `key` its code names, with
 * the values it takes, and the message `fallback` for a code the page has no message for. Only a number or a text of
 * the body goes into the message, never the admin's English (`error`). */
export interface Told {
  key: string | null;
  params: Record<string, string | number>;
  fallback: string;
}

/** What `coded` (`{code, ...fields}`: an admin's refusal, or a failure inside an answer, a session's `failed` or a
 * saved session's `unsaved`) says, its fallback by `status`: a refusal (4xx), a failure (any other status, or none:
 * inside an answer), or, with `unreached`, a request that got no answer. */
export function toldBy(coded: unknown, status: number | undefined, unreached = false): Told {
  const body = coded !== null && typeof coded === 'object' ? (coded as Record<string, unknown>) : {};
  const params: Record<string, string | number> = {};
  for (const [name, value] of Object.entries(body)) {
    if (name !== 'error' && name !== 'code' && (typeof value === 'number' || typeof value === 'string')) params[name] = value;
  }
  const code = body.code;
  const key = typeof code === 'string' && /^[a-z0-9]+(-[a-z0-9]+)*$/.test(code) ? `errors.${code}` : null;
  const fallback = unreached
    ? 'errors.generic.network'
    : status !== undefined && status >= 400 && status < 500
      ? 'errors.generic.refused'
      : 'errors.generic.failed';
  return { key, params, fallback };
}

/**
 * The address the admin is set up for (`ADMIN_ORIGIN`), when it refused a request because this page is open at another
 * one (`https://127.0.0.1:9443` for `https://localhost:9443`, another tunnel port, the server's IP); null for any other
 * error. Only an `https://` origin is taken, as the page links to it.
 */
export function wrongOrigin(e: unknown): string | null {
  const { code, origin } = bodyOf(e);
  const valid = typeof origin === 'string' && /^https:\/\/[^/?#\\@\s]+$/.test(origin);
  return statusOf(e) === 403 && code === 'wrong-origin' && valid ? origin : null;
}

export interface Status {
  enrolled: boolean;
  /** A passkey is added and this address can have them: the login offers one. */
  passkey?: boolean;
  /** Set when this installation is the public demo. */
  demo?: DemoInfo | null;
}
export interface DemoInfo {
  sites: string[];
  seconds: number;
  /** The pages that may show the demo in a frame, and pick its demo (see `embed.ts`). */
  embedders: string[];
}
/** A demo visitor's turn: their session, the demo it shows, and the seconds left. */
export interface Visit {
  session: string;
  site: string;
  seconds_left: number;
}
/** The logged-in browser, as `GET /api/me` answers: its CSRF token and, for a demo visitor, their visit (null once over). */
interface Me {
  csrf: string;
  demo?: { visit: Visit | null } | null;
}
export const me = (): Promise<Me> => api<Me>('GET', '/api/me');
/** The demo visitor's visit as the admin has it now (a switch in another tab moves it), or null when it is over. */
export const currentVisit = async (): Promise<Visit | null> => (await me()).demo?.visit ?? null;
/**
 * Ends this browser's login: the request goes with the token, which is forgotten at once. A failed logout changes
 * nothing the page does next (it leaves or reloads either way), so it resolves all the same.
 */
export function logout(): Promise<void> {
  const sent = api('POST', '/api/logout').then(
    () => undefined,
    () => undefined,
  );
  setCsrf('');
  return sent;
}
export interface SessionInfo {
  id: string;
  url: string;
  devtools: boolean;
  features: Features;
  /** The proxy it browses from: its name when the session started, and its id (admin/src/egresses.rs). */
  egress?: string | null;
  egress_id?: string | null;
  /** The saved profile it started from (admin/src/profiles.rs). */
  profile?: { id: string; name: string } | null;
  /** Why it failed, `{code, error, ...}` (see errors.ts, `saidOf`): it stopped, or stopping it is to be retried. */
  failed?: Record<string, unknown> | null;
}
/** An agent token as the admin lists it: never the token itself, which only its creation answers once. */
export interface TokenInfo {
  id: string;
  name: string;
  /** Unix seconds. */
  created: number;
  last_used: number | null;
  /** Revoked, but its revocation could not be written yet: refused all the same. */
  revoked: boolean;
}

/**
 * The session a watch link opens: `#session=<id>` on the admin page (the agent API's `watch_url`), or null.
 */
export function linkedSession(hash: string): string | null {
  return /^#session=([0-9a-f]{12})$/.exec(hash)?.[1] ?? null;
}

/** Writes `text` to the person's clipboard: whether the browser allowed it. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/** Each language's and unit's size formatter, made once: making one costs some 40 times formatting with it, and a
 * transfer's progress formats every list entry on each update. */
const sizeFormats = new Map<string, Intl.NumberFormat>();

/** A size as a person reads it in `locale`: bytes, kB, MB or GB, in that language's way of writing them. */
export function formatSize(bytes: number, locale: string): string {
  const [value, unit, digits] = bytes < 1e3 ? [bytes, 'byte', 0] : bytes < 1e6 ? [bytes / 1e3, 'kilobyte', 0] : bytes < 1e9 ? [bytes / 1e6, 'megabyte', 1] : [bytes / 1e9, 'gigabyte', 1];
  const key = `${locale} ${unit}`;
  let formatter = sizeFormats.get(key);
  if (!formatter) sizeFormats.set(key, (formatter = new Intl.NumberFormat(locale, { style: 'unit', unit, unitDisplay: 'short', maximumFractionDigits: digits })));
  return formatter.format(value);
}

/** What a refusal for too many attempts (429) says: its message's key, translated where it shows. */
export const TOO_MANY_ATTEMPTS = 'errors.too-many-attempts';
/** What a password check that failed says: its message's key, translated where it shows. */
export const NOT_YOUR_PASSWORD = 'errors.wrong-password';

type Area = 'localStorage' | 'sessionStorage';
/**
 * A per-browser convenience setting (this tab's alone in `sessionStorage`); storage may be unavailable, and then the
 * default applies.
 */
export function remembered(key: string, area: Area = 'localStorage'): string | null {
  try {
    return window[area].getItem(key);
  } catch {
    return null;
  }
}

/** Remembers `value` under `key`, or forgets it (null). */
export function remember(key: string, value: string | null, area: Area = 'localStorage'): void {
  try {
    if (value === null) window[area].removeItem(key);
    else window[area].setItem(key, value);
  } catch {
    // Not remembered; the setting still applies to this page.
  }
}
