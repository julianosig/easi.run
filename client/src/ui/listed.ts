// The running sessions as this page last listed them (the sessions page, a session's sidebar): a session opened from
// there starts from what is known of it (its features, its chat) while it is listed again, instead of waiting for that.

import type { SessionInfo } from './api.ts';

let last = new Map<string, SessionInfo>();

/** Keeps `sessions` as the latest listing. */
export function rememberListed(sessions: SessionInfo[]): void {
  last = new Map(sessions.map((s) => [s.id, s]));
}

/** What the latest listing said of session `id`, if it listed it. */
export const known = (id: string): SessionInfo | null => last.get(id) ?? null;
