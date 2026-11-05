// The page's look, shared by the admin and the site: the system's, unless a person picked light or dark, which this
// browser remembers. public/theme.js applies a pick before the page first paints; the colours are in app.css.
export const THEMES = ['system', 'light', 'dark'] as const;
export type Theme = (typeof THEMES)[number];

const KEY = 'theme';
const root = () => document.documentElement;

export function chosenTheme(): Theme {
  try {
    const kept = localStorage.getItem(KEY);
    return kept === 'light' || kept === 'dark' ? kept : 'system';
  } catch {
    return 'system';
  }
}

export function chooseTheme(theme: Theme): void {
  if (theme === 'system') delete root().dataset.theme;
  else root().dataset.theme = theme;
  try {
    if (theme === 'system') localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, theme);
  } catch {
    // Not remembered; it still applies to this page.
  }
}

/** Whether the page shows dark now, for what its colours can't reach (a terminal's). */
export function isDark(): boolean {
  const picked = root().dataset.theme;
  return picked ? picked === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches;
}

/** Calls `changed` whenever the look changes, by a pick or the system's, until the function it returns is called. */
export function watchTheme(changed: () => void): () => void {
  const system = matchMedia('(prefers-color-scheme: dark)');
  const picks = new MutationObserver(changed);
  picks.observe(root(), { attributes: true, attributeFilter: ['data-theme'] });
  system.addEventListener('change', changed);
  return () => {
    picks.disconnect();
    system.removeEventListener('change', changed);
  };
}
