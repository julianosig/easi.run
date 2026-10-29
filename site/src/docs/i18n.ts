// The docs' language: the address fixes it (/ English, /<locale>/ each other), set once before they mount (main.ts).
// Their messages are their own (messages/, i18n/vite.ts as virtual:docs-messages/<locale>): the landing never loads
// them, nor they the landing's.
import { type ArgsOf, type Catalog, format, type Params } from '../../../i18n/format.ts';
import { type Locale, pathOf, SOURCE } from '../i18n/locale.ts';
import { ORIGIN } from '../lib/site.ts';
import type { MessageParams } from './keys.ts';
import { OTHERS } from 'virtual:docs-messages';

let current: { locale: Locale; catalog: Catalog } = { locale: SOURCE, catalog: {} };

export function use(locale: Locale, catalog: Catalog): void {
  current = { locale, catalog };
}

export const locale = (): Locale => current.locale;

/** A message's text as written, markup included (Rich.svelte parses it); its key where there is none. */
export function template(key: string): string {
  const message = Object.hasOwn(current.catalog, key) ? current.catalog[key] : undefined;
  return typeof message === 'string' ? message : key;
}

/** A plain message in the docs' language, its parameters filled. */
export function t<K extends keyof MessageParams>(key: K, ...[params]: ArgsOf<MessageParams, K>): string {
  return say(key as string, params as Params | undefined);
}

/** `t` for a key made from data (a setting's, a route's). */
export function say(key: string, params?: Params): string {
  const message = Object.hasOwn(current.catalog, key) ? current.catalog[key] : undefined;
  return message === undefined ? key : format(message, params ?? {}, current.locale);
}

/** `url` on easi.run itself, in the docs' language: https://easi.run/#demo is https://easi.run/fr/#demo in French. */
export function localized(url: string): string {
  const home = `${ORIGIN}/`;
  return url === home || url.startsWith(`${home}#`) ? `${ORIGIN}${pathOf(current.locale)}${url.slice(home.length)}` : url;
}

/** Each language's catalog, one file each. */
export const CATALOGS: Record<Locale, () => Promise<{ default: Catalog }>> = {
  en: () => import('virtual:docs-messages/en'),
  ...OTHERS,
};
