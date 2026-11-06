// How a person likes a session's chat laid out (Chat.svelte): where it sits, how much of the window it takes in each place,
// and how much of the screen shows through it. This browser's own convenience, for every session here.
import { remember, remembered } from './api.ts';

export type ChatView = 'bottom' | 'side';

/** The most of the window it may take, in either place. */
export const MAX_SHARE = 0.7;
/** The least it takes, in pixels: its header and some 8 lines of terminal at the bottom; at the side, all of its header's
 *  controls with the longest product's name, and a little of the account's. */
export const MIN_PIXELS: Record<ChatView, number> = { bottom: 220, side: 420 };
/** The most of the screen that may show through it, in percent. */
export const MAX_TRANSPARENCY = 70;
/** Below this width there is no room for the side: it sits at the bottom. */
export const SIDE_FROM_WIDTH = 640;

export interface ChatPanel {
  view: ChatView;
  /** The share of the window it takes, in each place. */
  shares: Record<ChatView, number>;
  transparency: number;
}

const KEY = 'chat-panel';
const DEFAULTS: ChatPanel = { view: 'bottom', shares: { bottom: 0.5, side: 0.4 }, transparency: 5 };

/** `share` within what `view` may take of a window `pixels` long along its resizing direction. */
export function clampShare(share: number, view: ChatView, pixels: number): number {
  return Math.min(MAX_SHARE, Math.max(share, MIN_PIXELS[view] / pixels));
}

export function clampTransparency(percent: number): number {
  return Math.min(MAX_TRANSPARENCY, Math.max(0, Math.round(percent)));
}

const fraction = (value: unknown, otherwise: number) => (typeof value === 'number' && value > 0 && value <= 1 ? value : otherwise);

/** What `text` remembers, whatever of it is usable; the defaults for the rest. */
export function parsePanel(text: string | null): ChatPanel {
  let kept: Partial<ChatPanel> = {};
  try {
    kept = (JSON.parse(text ?? '{}') ?? {}) as Partial<ChatPanel>;
  } catch {
    // Not what this page wrote: as if nothing were remembered.
  }
  return {
    view: kept.view === 'side' ? 'side' : 'bottom',
    shares: {
      bottom: fraction(kept.shares?.bottom, DEFAULTS.shares.bottom),
      side: fraction(kept.shares?.side, DEFAULTS.shares.side),
    },
    transparency: typeof kept.transparency === 'number' ? clampTransparency(kept.transparency) : DEFAULTS.transparency,
  };
}

const OPEN_KEY = 'chat-panel-open';
/** Whether the chat was left showing, for a session here that has one when it next opens. */
export const chatWasOpen = (): boolean => remembered(OPEN_KEY) === '1';
export const rememberChatOpen = (open: boolean): void => remember(OPEN_KEY, open ? '1' : '0');

export const loadPanel = (): ChatPanel => parsePanel(remembered(KEY));
export const savePanel = (panel: ChatPanel): void => remember(KEY, JSON.stringify(panel));
