// The admin's first screen, where the person creates the password: the setup link's token, and the password rules
// the admin applies (admin/src/auth.rs), checked here first so the screen can say what is missing as they type.

/** The fewest and the most characters a password may have, counted as the admin counts them: code points. */
export const MIN_PASSWORD = 12;
export const MAX_PASSWORD = 1024;

/**
 * The enrollment token in a setup link (`https://host:port/#setup=<token>`), in its fragment alone, or given bare,
 * spaces around it ignored; null for anything else. A token is 32 random bytes in unpadded base64url, as the admin
 * draws it (`random_token`).
 */
export function setupToken(text: string): string | null {
  const trimmed = text.trim();
  const token = /(?:^|#)setup=([^#]*)$/.exec(trimmed)?.[1] ?? trimmed;
  return /^[A-Za-z0-9_-]{43}$/.test(token) ? token : null;
}

/** What keeps a new password from being sent: too short, too long, or a confirmation that differs. */
export type Problem = 'short' | 'long' | 'different';

export interface PasswordCheck {
  /** The characters still needed to reach the minimum (0 once it is reached), for the hint shown while typing. */
  missing: number;
  /** The confirmation differs from the password, and not only because it is still being typed (a beginning of it). */
  differs: boolean;
  /** Why the two cannot be sent yet, or null when they can. */
  problem: Problem | null;
}

export function checkPassword(password: string, confirm: string): PasswordCheck {
  const length = [...password].length;
  const missing = Math.max(0, MIN_PASSWORD - length);
  const problem = missing > 0 ? 'short' : length > MAX_PASSWORD ? 'long' : confirm !== password ? 'different' : null;
  return { missing, differs: confirm !== '' && !password.startsWith(confirm), problem };
}
