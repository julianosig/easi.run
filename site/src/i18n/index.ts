// The page's language: its address fixes it (/ English, /<locale>/ each other), and it is set once before the page
// renders, at the build for each language (src/render.ts), or before it hydrates in the browser (src/main.ts). `t`
// reads it; nothing changes it afterwards.
import type { Catalog } from '../../../i18n/format.ts';
import type { Locale } from '../../../i18n/locales.ts';
import { OTHERS } from 'virtual:messages';
import { catalogReader } from './catalog.ts';
import type { MessageParams } from './keys.ts';

export { AUTONYMS, choose, chosen, CHOSEN, LOCALES, type Locale, localeOf, match, pathOf, SOURCE } from './locale.ts';

// `has` is for names that come from elsewhere (the demo's), each said in English where the catalog has none. A key made
// from data (a comparison cell's) that it lacks fails the build's render, and shows as its key in a browser.
export const { use, locale, t, has, say } = catalogReader<MessageParams>((locale, key) => {
  if (import.meta.env.SSR) throw new Error(`${locale}: no message ${key}`);
  return key;
});

/** Each language's catalog, one file each (i18n/vite.ts). */
export const CATALOGS: Record<Locale, () => Promise<{ default: Catalog }>> = {
  en: () => import('virtual:messages/en'),
  ...OTHERS,
};
