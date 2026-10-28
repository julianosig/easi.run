// The logged-in admin's shapes, beside the landing's building blocks (ui.ts): its pages' icons, headings and rows.

import { ICONS } from './icons.ts';
import type { Page } from './route.ts';

/** Each page's icon in the sidebar. */
export const PAGE_ICONS: Record<Page, string> = { sessions: ICONS.sessions, agents: ICONS.agents, proxies: ICONS.proxies, security: ICONS.security };
/** A page's list, its rows set apart by space and a soft surface under the pointer, never a rule. */
export const rows = '-mx-3 space-y-1';
export const row = 'rounded-2xl px-3 transition hover:bg-wash focus-within:bg-wash';
/** A small uppercase label over a group, as the landing's section headers. */
export const groupLabel = 'text-xs font-semibold uppercase tracking-[0.12em] text-muted';
/** A small label beside a row's name or in its meta line; its tone says what kind. */
export const chip = 'inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-semibold';
export const CHIP_TONES = {
  neutral: 'bg-wash text-muted',
  accent: 'bg-accent-soft text-accent',
  good: 'bg-good/10 text-good',
  bad: 'bg-bad/10 text-bad',
};
/** A row's leading icon, in its badge. */
export const rowBadge = 'relative grid size-10 shrink-0 place-items-center rounded-xl bg-accent-soft text-accent';
