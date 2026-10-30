// The site's constants, kept apart from the comparison (content.ts) so a page that needs only these does not load it.
// The name changes here.
import { localeOf, pathOf } from '../i18n/locale.ts';

export const NAME = 'easi.run';

/** Where the site is, for the addresses search engines and link previews take. */
export const ORIGIN = 'https://easi.run';

/** The live demo the hero shows, on this same host: one visit per visitor, no password. */
export const DEMO = { url: '/demo/' };

/** The public repository; the docs the page cites live in it. */
export const REPO = 'https://github.com/julianosig/easi.run';

/** The landing's pages, each with its own path below its language's address (pathOf): the live demo first, the
 * features, the comparison. */
export const PAGES = { home: '', features: 'features/', compare: 'compare/' } as const;
export type Page = keyof typeof PAGES;

/** The page an address shows, its language's prefix aside: the first page for any other. */
export function pageOf(path: string): Page {
  const own = path.slice(pathOf(localeOf(path)).length).split('/')[0];
  return (Object.keys(PAGES) as Page[]).find((page) => PAGES[page] === `${own}/`) ?? 'home';
}
