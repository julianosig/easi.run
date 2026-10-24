// The landing's building blocks, as Tailwind classes, so every view uses the same few shapes.

import { Codec } from '../protocol.ts';

export const focusRing = 'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent';
export const button = `rounded-xl border border-line bg-paper px-4 py-2.5 text-[15px] font-semibold text-ink transition hover:border-ink/30 disabled:cursor-default disabled:opacity-50 ${focusRing}`;
export const primary = `rounded-xl bg-accent px-5 py-2.5 text-[15px] font-semibold text-white shadow-[0_8px_20px_-10px_rgb(36_87_245/0.8)] transition hover:bg-accent-strong disabled:cursor-default disabled:opacity-50 ${focusRing}`;
export const field =
  'w-full rounded-xl border border-line bg-paper px-4 py-3 text-[15px] text-ink placeholder:text-muted/70 focus:border-accent focus:outline-3 focus:outline-accent-soft';
export const label = 'mb-1.5 mt-4 block text-sm font-semibold';
export const card = 'rounded-2xl border border-line bg-paper p-6 shadow-[0_24px_60px_-40px_rgb(20_40_75/0.45)]';
export const panelTitle = 'mb-3 text-[15px] font-semibold tracking-tight';
/** What each codec is called on screen (no codec: the screen comes as lossless tiles only). */
export const CODEC_NAMES: Record<Codec, string> = { [Codec.None]: 'Lossless tiles', [Codec.Av1]: 'AV1', [Codec.H264]: 'H.264' };
