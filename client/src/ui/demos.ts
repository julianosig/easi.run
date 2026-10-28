// The public demo's pages, by their name in ADMIN_DEMO_SITES: a title and one line (message keys, which the page
// translates as it draws them), and an icon path (24-unit box).
import type { MessageParams } from '../i18n/keys.ts';

/** A demo's message key: the title or the line of a known page; an unknown page's name, which `t()` answers as is. */
export type DemoText = Extract<keyof MessageParams, `demo.${string}.${'title' | 'line'}`>;

export interface Demo {
  title: DemoText;
  line: DemoText;
  icon: string;
}

const DEMOS = new Map<string, Demo>(Object.entries<Demo>({
  takeover: { title: 'demo.takeover.title', line: 'demo.takeover.line', icon: 'M7 11V8a5 5 0 0 1 10 0v3M5 11h14v10H5zM12 15v2' },
  drag: { title: 'demo.drag.title', line: 'demo.drag.line', icon: 'M12 3v12M7 10l5 5 5-5M5 21h14' },
  upload: { title: 'demo.upload.title', line: 'demo.upload.line', icon: 'M12 21V9M7 14l5-5 5 5M5 3h14' },
  download: { title: 'demo.download.title', line: 'demo.download.line', icon: 'M12 3v12M7 10l5 5 5-5M4 17v4h16v-4' },
  sound: { title: 'demo.sound.title', line: 'demo.sound.line', icon: 'M4 9.5v5h3.5L12 18.5v-13L7.5 9.5zM16 9a4.2 4.2 0 0 1 0 6M18.6 6.4a8 8 0 0 1 0 11.2' },
  typing: { title: 'demo.typing.title', line: 'demo.typing.line', icon: 'M3 6h18v12H3zM7 10h.01M11 10h.01M15 10h.01M8 14h8' },
  clipboard: { title: 'demo.clipboard.title', line: 'demo.clipboard.line', icon: 'M9 4h6v3H9zM8 5.5H6v15h12v-15h-2M9 12h6M9 16h4' },
}));

/** An unknown page shows its name, and no line: `t()` gives back a key it has no message for. */
export const demoOf = (name: string): Demo => DEMOS.get(name) ?? { title: name as DemoText, line: '' as DemoText, icon: 'M5 12h14' };
