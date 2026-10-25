// The landing's building blocks, as Tailwind classes, so every view uses the same few shapes.

import { Codec } from '../protocol.ts';

export const focusRing = 'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent';
export const button = `rounded-xl border border-line bg-paper px-4 py-2.5 text-[15px] font-semibold text-ink transition hover:border-ink/30 disabled:cursor-default disabled:opacity-50 ${focusRing}`;
export const primary = `rounded-xl bg-accent px-5 py-2.5 text-[15px] font-semibold text-white shadow-[0_8px_20px_-10px_rgb(36_87_245/0.8)] transition hover:bg-accent-strong disabled:cursor-default disabled:opacity-50 ${focusRing}`;
export const field =
  'w-full rounded-xl border border-line bg-paper px-4 py-3 text-[15px] text-ink placeholder:text-muted/70 focus:border-accent focus:outline-3 focus:outline-accent-soft';
/** A field for a 6-digit code: its digits large, spaced and centred. */
export const codeField = field.replace('text-[15px]', 'text-center font-mono text-xl tracking-[0.35em]');
export const label = 'mb-1.5 mt-4 block text-sm font-semibold';
export const card = 'rounded-2xl border border-line bg-paper p-6 shadow-[0_24px_60px_-40px_rgb(20_40_75/0.45)]';
/** The top bar's icons (see `Bar`): paths in 24 units, drawn as strokes, as the landing's menu draws its own. */
export const BAR_ICONS = {
  sessions: 'M4 5h16v14H4zM4 9h16M7 7h.01M10 7h.01',
  security: 'M12 3l7 3v5c0 4.5-3 8.4-7 10-4-1.6-7-5.5-7-10V6zM9 12l2 2 4-4',
  close: 'M4 5h16v14H4zM9.5 11.5l5 5M14.5 11.5l-5 5',
};
/** The icons of the cards before a login (see `Gate`), drawn as the top bar's are. */
export const GATE_ICONS = {
  warning: 'M12 4l9 15H3zM12 10v4M12 17h.01',
  phone: 'M7 3h10v18H7zM11 17.5h2',
  passkey: 'M9 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7zM3 20c0-3.3 2.7-5.5 6-5.5 1.3 0 2.5.3 3.5.9M17 13a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5zM19 17.5l2 2.5M20.5 19.5l-1 1',
  shield: BAR_ICONS.security,
};
/** The headline of a screen before a login (see `Gate`), at the landing's scale, its lead and its main button. */
export const gateTitle = 'text-[clamp(2.25rem,3.1vw,3rem)] font-semibold leading-[1.04] tracking-[-0.035em] text-balance';
/** The headline of a step inside one of those screens (adding the app, saving the recovery codes). */
export const gateStepTitle = 'text-[clamp(1.875rem,2.4vw,2.25rem)] font-semibold leading-[1.08] tracking-[-0.03em] text-balance';
/** A step's headline where it sits in a page after a login. */
export const stepTitle = 'text-xl font-semibold tracking-[-0.02em]';
export const gateLead = 'mt-3 text-[17px] leading-relaxed text-pretty text-muted';
export const gatePrimary = primary.replace('py-2.5', 'py-3.5').replace('text-[15px]', 'text-base');
/** One of the icons above in its badge: a rounded square tinted blue, or red for what went wrong. */
export const badge = 'grid shrink-0 place-items-center rounded-xl bg-accent-soft text-accent';
export const panelTitle = 'mb-3 text-[15px] font-semibold tracking-tight';
/** What each codec is called on screen (no codec: the screen comes as lossless tiles only). */
export const CODEC_NAMES: Record<Codec, string> = { [Codec.None]: 'Lossless tiles', [Codec.Av1]: 'AV1', [Codec.H264]: 'H.264' };
