// Which language a page of the site is in, and the visitor's pick, for the landing and the docs alike (each loads its
// own messages): / is English, /<locale>/ each other language.
import { isLocale, type Locale, SOURCE } from '../../../i18n/locales.ts';

export { AUTONYMS, LOCALES, type Locale, match, SOURCE } from '../../../i18n/locales.ts';

/** The address of a page in `locale`: / for English, /<locale>/ for the others. */
export const pathOf = (locale: Locale): string => (locale === SOURCE ? '/' : `/${locale}/`);

/** The language an address is in: its first segment's, English when it names none. */
export function localeOf(path: string): Locale {
  const first = path.split('/')[1] ?? '';
  return isLocale(first) && first !== SOURCE ? first : SOURCE;
}

/** Where this browser keeps the language its visitor picked on the site (each host keeps its own). */
export const CHOSEN = 'easi-run:site-locale';

/** The language this browser's visitor picked, if any. */
export function chosen(): Locale | null {
  try {
    const value = localStorage.getItem(CHOSEN);
    return isLocale(value) ? value : null;
  } catch {
    return null;
  }
}

/** Remembers `locale` as the visitor's pick, where the browser lets it. */
export function choose(locale: Locale): void {
  try {
    localStorage.setItem(CHOSEN, locale);
  } catch {
    // A private window, say: the pick holds for this visit only.
  }
}
