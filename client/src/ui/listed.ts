// The running sessions as this page last listed them (the sessions page, a session's sidebar): a session opened from
// there starts from what is known of it (its features, its chat) while it is listed again, instead of waiting for that.

import type { SessionInfo, TokenInfo } from './api.ts';
import type { ProfileList } from './profiles.ts';
import type { Proxy } from './proxies.ts';

let last = new Map<string, SessionInfo>();

/** What a session's sidebar last read: its list, and what its start dialog needs. */
export interface Listing {
  sessions: SessionInfo[];
  profiles: ProfileList;
  tokens: TokenInfo[];
  egresses: Proxy[];
}
let listing: Listing | null = null;

/** Keeps what a session's sidebar read, for the next session's sidebar to start from: going from one session to
 * another mounts it anew, and a list that began empty each time would be gone until it was read again. */
export function rememberListing(read: Listing): void {
  listing = read;
  rememberListed(read.sessions);
}

/** What the sidebar last read, if it did since this login. */
export const lastListing = (): Listing | null => listing;

/** Forgets every listing: the login is over, and the next one reads its own. */
export function forgetListed(): void {
  last = new Map();
  listing = null;
}

/** Keeps `sessions` as the latest listing. */
export function rememberListed(sessions: SessionInfo[]): void {
  last = new Map(sessions.map((s) => [s.id, s]));
}

/** What the latest listing said of session `id`, if it listed it. */
export const known = (id: string): SessionInfo | null => last.get(id) ?? null;

/** Two letters that tell a session apart at a glance: a name's first two words', or its first two letters; a site's
 *  own name (what comes before its top-level domain) stands for its address. Whole characters, never half of one. */
export function initials(name: string): string {
  const labels = name.split('.');
  const own = labels.length > 1 && !name.includes(' ') ? labels[labels.length - 2]! : name;
  const words = own.split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  const first = (word: string): string => [...word][0]!;
  return (words.length > 1 ? first(words[0]!) + first(words[1]!) : [...(words[0] ?? '?')].slice(0, 2).join('')).toUpperCase();
}
