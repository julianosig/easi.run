// The admin API, as admin/src/api.rs serves it: same-origin cookies, and the session's CSRF token on every
// state-changing request. Errors carry the HTTP status, so a view can tell a lapsed login (401) or a busy demo (409).

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
    const why = (data as { error?: unknown }).error;
    throw Object.assign(new Error(typeof why === 'string' ? why : response.statusText), {
      status: response.status,
      body: data as Record<string, unknown>,
    });
  }
  return data as T;
}

export const statusOf = (e: unknown): number | undefined => (e as ApiError).status;
export const messageOf = (e: unknown): string => (e as Error).message;
export const bodyOf = (e: unknown): Record<string, unknown> => (e as ApiError).body ?? {};
/**
 * The address the admin is set up for (`ADMIN_ORIGIN`), when it refused a request because this page is open at another
 * one (`https://127.0.0.1:9443` for `https://localhost:9443`, another tunnel port, the server's IP); null for any other
 * error. Only an `https://` origin is taken, as the page links to it.
 */
export function wrongOrigin(e: unknown): string | null {
  const { reason, origin } = bodyOf(e);
  const valid = typeof origin === 'string' && /^https:\/\/[^/?#\\@\s]+$/.test(origin);
  return statusOf(e) === 403 && reason === 'wrong-origin' && valid ? origin : null;
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
  settings: EncoderSettings;
  network: NetworkConditions;
  egress?: string | null;
  /** The saved profile it started from (admin/src/profiles.rs). */
  profile?: { id: string; name: string } | null;
  failed?: string | null;
}
/** A session's encoder settings, as the admin keeps them (admin/src/app.rs `Settings`). */
export interface EncoderSettings {
  codec: 'auto' | 'av1' | 'h264' | 'tiles_only';
  bitrate_kbps: number;
  max_fps: number;
}
/** The network conditions simulated on a session's viewer streams (admin/src/app.rs `Network`). */
export interface NetworkConditions {
  latency_ms: number;
  loss_percent: number;
  bandwidth_kbps: number;
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

/** An upstream proxy a session may leave through, as the deployment configured it (never its address). */
export interface Egress {
  name: string;
  timezone: string;
  languages: string[];
}
interface ContainerInfo {
  status?: string;
  cpu_percent?: number | null;
  memory_bytes?: number | null;
}
/** The controller's view of a session's containers, or why it could not say. */
export interface Inspection {
  containers?: Record<string, ContainerInfo>;
  error?: string;
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KiB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
}

/** A per-browser convenience setting; storage may be unavailable, and then the default applies. */
export function remembered(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function remember(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Not remembered; the setting still applies to this page.
  }
}
