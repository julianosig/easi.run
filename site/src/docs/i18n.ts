// The docs' language: the address fixes it (/ English, /<locale>/ each other), set once before they mount (main.ts).
// Their messages are their own (messages/, i18n/vite.ts as virtual:docs-messages/<locale>): the landing never loads
// them, nor they the landing's.
import type { Catalog } from '../../../i18n/format.ts';
import { catalogReader } from '../i18n/catalog.ts';
import { type Locale, pathOf } from '../i18n/locale.ts';
import { ORIGIN } from '../lib/site.ts';
import type { MessageParams } from './keys.ts';
import { OTHERS } from 'virtual:docs-messages';

// A key the docs' catalog lacks (a setting's, a route's) reads as itself.
const reader = catalogReader<MessageParams>((_, key) => key);
export const { use, locale, t, say } = reader;

/** A message's text as written, markup included (Rich.svelte parses it); its key where there is none. */
export function template(key: string): string {
  const message = reader.message(key);
  return typeof message === 'string' ? message : key;
}

/** `url` on easi.run itself, in the docs' language: https://easi.run/#demo is https://easi.run/fr/#demo in French. */
export function localized(url: string): string {
  const home = `${ORIGIN}/`;
  return url === home || url.startsWith(`${home}#`) ? `${ORIGIN}${pathOf(locale())}${url.slice(home.length)}` : url;
}

/** Each language's catalog, one file each. */
export const CATALOGS: Record<Locale, () => Promise<{ default: Catalog }>> = {
  en: () => import('virtual:docs-messages/en'),
  ...OTHERS,
};
