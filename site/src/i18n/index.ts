// The page's language: its address fixes it (/ English, /<locale>/ each other), and it is set once before the page
// renders, at the build for each language (src/render.ts), or before it hydrates in the browser (src/main.ts). `t`
// reads it; nothing changes it afterwards.
import { type Catalog, format, type Params } from '../../../i18n/format.ts';
import { type Locale, SOURCE } from '../../../i18n/locales.ts';
import type { MessageParams } from './keys.ts';

export { AUTONYMS, choose, chosen, CHOSEN, LOCALES, type Locale, localeOf, match, pathOf, SOURCE } from './locale.ts';

let current: { locale: Locale; catalog: Catalog } = { locale: SOURCE, catalog: {} };




/** Sets the page's language and its catalog, before it renders. */
export function use(locale: Locale, catalog: Catalog): void {
  current = { locale, catalog };
}

/** The page's language. */
export const locale = (): Locale => current.locale;

type Args<K extends keyof MessageParams> = MessageParams[K] extends Record<string, never> ? [] : [MessageParams[K]];

/** The message `key` in the page's language, its parameters filled. */
export function t<K extends keyof MessageParams>(key: K, ...[params]: Args<K>): string {
  return say(key, params as Params | undefined);
}

/** Whether the page's catalog has `key`: for names that come from elsewhere (the demo's), each said in English
 * where it has none. */
export const has = (key: string): boolean => Object.hasOwn(current.catalog, key);

/** `t` for a key made from data (a comparison cell's): one the catalog lacks fails the build's render and shows as
 * its key in a browser. */
export function say(key: string, params?: Params): string {
  if (!Object.hasOwn(current.catalog, key)) {
    if (import.meta.env.SSR) throw new Error(`${current.locale}: no message ${key}`);
    return key;
  }
  return format(current.catalog[key]!, params ?? {}, current.locale);
}



/** Each language's catalog, one file each (i18n/vite.ts). */
export const CATALOGS: Record<Locale, () => Promise<{ default: Catalog }>> = {
  en: () => import('virtual:messages/en'),
  'pt-BR': () => import('virtual:messages/pt-BR'),
  es: () => import('virtual:messages/es'),
  'zh-CN': () => import('virtual:messages/zh-CN'),
  hi: () => import('virtual:messages/hi'),
  fr: () => import('virtual:messages/fr'),
  de: () => import('virtual:messages/de'),
  ja: () => import('virtual:messages/ja'),
  ru: () => import('virtual:messages/ru'),
  id: () => import('virtual:messages/id'),
  ko: () => import('virtual:messages/ko'),
};
