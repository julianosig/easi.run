// The logged-in admin's shapes, beside the landing's building blocks (ui.ts): its pages' icons, headings and rows.

import { button, focusRing, primary } from './ui.ts';
import type { Page } from './route.ts';

/** Each page's icon in the sidebar: paths in 24 units, drawn as strokes. */
export const PAGE_ICONS: Record<Page, string> = {
  sessions: 'M4 5h16v14H4zM4 9h16M7 7h.01M10 7h.01',
  agents: 'M4 3l7 17 2.5-7.5L21 10zM14 14l5 5',
  proxies: 'M12 21s-6.5-5.6-6.5-10.5a6.5 6.5 0 0 1 13 0C18.5 15.4 12 21 12 21zM12 12.5a2 2 0 1 0 0-4 2 2 0 0 0 0 4z',
  security: 'M12 3l7 3v5c0 4.5-3 8.4-7 10-4-1.6-7-5.5-7-10V6zM9 12l2 2 4-4',
};
export const PAGE_NAMES: Record<Page, string> = { sessions: 'Sessions', agents: 'Agents', proxies: 'Proxies', security: 'Security' };
export const LOG_OUT_ICON = 'M14 4h4a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-4M10 8l-4 4 4 4M6 12h10';
export const PLUS_ICON = 'M12 5v14M5 12h14';
/** A page's headline, and the one subtle line under it. */
export const pageTitle = 'text-[1.75rem] font-semibold leading-tight tracking-[-0.03em]';
export const pageHelp = 'mt-1.5 max-w-2xl text-sm leading-relaxed text-muted';
/** A page's list, its rows set apart by space and a soft surface under the pointer, never a rule. */
export const rows = '-mx-3 space-y-1';
export const row = 'rounded-2xl px-3 transition hover:bg-wash focus-within:bg-wash';
/** A small uppercase label over a group, as the landing's section headers. */
export const groupLabel = 'text-xs font-semibold uppercase tracking-[0.12em] text-muted';
/** One action in a `Menu`. */
export const menuItem = `flex w-full items-center gap-2.5 rounded-xl px-3 py-2 text-left text-sm font-semibold text-ink transition hover:bg-wash disabled:cursor-default disabled:opacity-45 disabled:hover:bg-transparent ${focusRing}`;
/** The forms' icons, drawn as the sidebar's. */
export const FORM_ICONS = {
  website: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3.5 9h17M3.5 15h17M12 3c2.4 2.6 3.6 5.6 3.6 9s-1.2 6.4-3.6 9c-2.4-2.6-3.6-5.6-3.6-9S9.6 5.6 12 3z',
  persistent: 'M6 4h12v16l-6-4-6 4z',
  name: 'M4 12V5h7l9 9-7 7zM8.5 8.5h.01',
  devtools: 'M8 8l-4 4 4 4M16 8l4 4-4 4M13.5 5l-3 14',
  key: 'M14.5 4a5.5 5.5 0 1 1-4.9 8L4 17.6V20h3v-2h2v-2h2l1.2-1.2A5.5 5.5 0 0 1 14.5 4zM16 8h.01',
  lock: 'M6 11h12v9H6zM9 11V8a3 3 0 0 1 6 0v3',
  rename: 'M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4',
  terminal: 'M4 5h16v14H4zM8 10l3 2-3 2M13 15h3',
  start: 'M8 5.5v13l10.5-6.5z',
};
/** A small label beside a row's name or in its meta line; its tone says what kind. */
export const chip = 'inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-semibold';
export const CHIP_TONES = {
  neutral: 'bg-wash text-muted',
  accent: 'bg-accent-soft text-accent',
  good: 'bg-good/10 text-good',
  bad: 'bg-bad/10 text-bad',
  warn: 'bg-you/10 text-[#b5452b]',
};
/** A row's leading icon, in its badge. */
export const rowBadge = 'relative grid size-10 shrink-0 place-items-center rounded-xl bg-accent-soft text-accent';
/** A row's own action (Watch, Resume): the secondary button, a size down. */
export const rowAction = `${button.replace('px-4 py-2.5 text-[15px]', 'px-3.5 py-2 text-sm')} inline-flex items-center gap-2`;
export const rowPrimary = `${primary.replace('px-5 py-2.5 text-[15px]', 'px-4 py-2 text-sm')} inline-flex items-center gap-2`;
