// The saved profiles, as admin/src/profiles.rs lists them: what a profile's row says, and who besides the
// administrator may start sessions from it (agent tokens, by id, and the local MCP clients).

import { formatSize, type TokenInfo } from './api.ts';

/** A saved profile as `GET /api/profiles` lists it: never its key or its archive. */
export interface ProfileInfo {
  id: string;
  name: string;
  /** Token ids, and `local` for the local MCP clients. */
  allowed: string[];
  /** Unix seconds. */
  created: number;
  /** When a session from it last ended with it saved, and the archive's size then. */
  used: number | null;
  size: number | null;
  /** The session using it now. */
  session: string | null;
  /** Why its last save failed, since the server started. */
  unsaved: string | null;
}
export interface ProfileList {
  /** Whether a password unlocked them since the server started. */
  unlocked: boolean;
  profiles: ProfileInfo[];
  max: number;
}

/** How the allowed list names the local MCP clients (`admin mcp`). */
export const LOCAL = 'local';

/** Who besides the administrator may use `profile`, by name, as the page lists them: the local MCP clients, then each
 * token that still exists. */
export function allowedNames(profile: ProfileInfo, tokens: TokenInfo[]): string[] {
  const names = profile.allowed.includes(LOCAL) ? ['Local MCP'] : [];
  for (const token of tokens) if (profile.allowed.includes(token.id) && !token.revoked) names.push(token.name);
  return names;
}

/** The line under a profile's name: when it was last saved and how large, and whether a session uses it now. */
export function profileSummary(profile: ProfileInfo, when: (seconds: number) => string): string {
  const saved =
    profile.used === null ? 'Not saved yet' : `Saved ${when(profile.used)}${profile.size === null ? '' : ` · ${formatSize(profile.size)}`}`;
  return profile.session === null ? saved : `${saved} · in use now`;
}

/** `allowed` with `entry` in it or not, `local` first and the rest as they were. */
export function toggled(allowed: string[], entry: string, on: boolean): string[] {
  const rest = allowed.filter((a) => a !== entry);
  if (!on) return rest;
  return entry === LOCAL ? [LOCAL, ...rest] : [...rest, entry];
}
