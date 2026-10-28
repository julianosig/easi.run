// The landing's building blocks, as Tailwind classes, so every view uses the same few shapes.

import { Codec } from '../protocol.ts';

export const focusRing = 'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent';
/** A button that reads as a link: a small action beside text. */
export const textLink = `text-sm font-semibold text-accent hover:underline rounded ${focusRing}`;

/** What a button is for: the one main action of its view (primary), any other (secondary), one that removes or ends
 * something (danger), or a quiet one beside content (ghost; alone as an icon, tinted by its tone). */
export type ButtonVariant = 'primary' | 'secondary' | 'danger' | 'ghost';
/** A ghost button's colour: what its icon means at a glance (edit is the accent, delete and close the danger). */
export type ButtonTone = 'neutral' | 'accent' | 'danger';
/** sm in rows and bars, md in forms and dialogs, lg the full-width action of a screen before a login. */
export type ButtonSize = 'sm' | 'md' | 'lg';

const BUTTON = `inline-flex shrink-0 items-center justify-center gap-2 rounded-xl font-semibold transition disabled:cursor-default disabled:opacity-50 ${focusRing}`;
const VARIANTS: Record<ButtonVariant, string> = {
  primary: 'bg-accent text-white shadow-[0_8px_20px_-10px_rgb(36_87_245/0.8)] hover:bg-accent-strong',
  secondary: 'border border-line bg-paper text-ink hover:border-ink/30',
  danger: 'border border-bad/25 bg-paper text-bad hover:border-bad/50 hover:bg-bad/5',
  ghost: '',
};
const TONES: Record<ButtonTone, string> = {
  neutral: 'text-muted hover:bg-wash hover:text-ink',
  accent: 'text-accent hover:bg-accent-soft',
  danger: 'text-bad hover:bg-bad/10',
};
const SIZES: Record<ButtonSize, string> = { sm: 'px-3.5 py-2 text-sm', md: 'px-4 py-2.5 text-[15px]', lg: 'w-full px-5 py-3.5 text-base' };
/** An icon alone: a square, as tall as a button of its size with words. */
const SQUARES: Record<ButtonSize, string> = { sm: 'size-9', md: 'size-10', lg: 'size-12' };

/** A button's classes: the one place every button's look is set (Button.svelte; an anchor that acts as one). */
export const buttonClass = (variant: ButtonVariant = 'secondary', size: ButtonSize = 'md', iconOnly = false, tone: ButtonTone = 'neutral') =>
  `${BUTTON} ${variant === 'ghost' ? TONES[tone] : VARIANTS[variant]} ${iconOnly ? SQUARES[size] : SIZES[size]}`;
export const field =
  'w-full rounded-xl border border-line bg-paper px-4 py-3 text-[15px] text-ink placeholder:text-muted/70 focus:border-accent focus:outline-3 focus:outline-accent-soft';
/** A field for a 6-digit code: its digits large, spaced and centred. */
export const codeField = field.replace('text-[15px]', 'text-center font-mono text-xl tracking-[0.35em]');
export const label = 'mb-1.5 mt-4 block text-sm font-semibold';
/** The headline of a screen before a login (see `Gate`), at the landing's scale, its lead and its main button. */
export const gateTitle = 'text-[clamp(2.25rem,3.1vw,3rem)] font-semibold leading-[1.04] tracking-[-0.035em] text-balance';
/** The headline of a step inside one of those screens (adding the app, saving the recovery codes). */
export const gateStepTitle = 'text-[clamp(1.875rem,2.4vw,2.25rem)] font-semibold leading-[1.08] tracking-[-0.03em] text-balance';
/** A step's headline where it sits in a page after a login. */
export const stepTitle = 'text-xl font-semibold tracking-[-0.02em]';
export const gateLead = 'mt-3 text-[17px] leading-relaxed text-pretty text-muted';
/** What each video codec is called on screen (no codec, lossless tiles only, is a message of the session's). */
export const CODEC_NAMES: Partial<Record<Codec, string>> = { [Codec.Av1]: 'AV1', [Codec.H264]: 'H.264' };
