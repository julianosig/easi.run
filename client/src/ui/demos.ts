// The public demo's pages, by their name in ADMIN_DEMO_SITES: a title, one line, and an icon path (24-unit box).
export interface Demo {
  title: string;
  line: string;
  icon: string;
}

const DEMOS: Record<string, Demo> = {
  takeover: { title: 'Take over', line: 'Sign in with a password and a 2FA code', icon: 'M7 11V8a5 5 0 0 1 10 0v3M5 11h14v10H5zM12 15v2' },
  drag: { title: 'Drag & drop', line: 'Drop a file from your computer on the page', icon: 'M12 3v12M7 10l5 5 5-5M5 21h14' },
  upload: { title: 'Upload', line: 'The page opens your own file picker', icon: 'M12 21V9M7 14l5-5 5 5M5 3h14' },
  download: { title: 'Download', line: 'Files land in your Downloads folder', icon: 'M12 3v12M7 10l5 5 5-5M4 17v4h16v-4' },
  sound: { title: 'Sound', line: 'Play a jingle; hear it from my server', icon: 'M4 9.5v5h3.5L12 18.5v-13L7.5 9.5zM16 9a4.2 4.2 0 0 1 0 6M18.6 6.4a8 8 0 0 1 0 11.2' },
  typing: { title: 'Typing', line: 'Race the words-per-minute counter', icon: 'M3 6h18v12H3zM7 10h.01M11 10h.01M15 10h.01M8 14h8' },
  clipboard: { title: 'Copy & paste', line: 'Both ways, page and your computer', icon: 'M9 4h6v3H9zM8 5.5H6v15h12v-15h-2M9 12h6M9 16h4' },
};

export const demoOf = (name: string): Demo => DEMOS[name] ?? { title: name, line: '', icon: 'M5 12h14' };
