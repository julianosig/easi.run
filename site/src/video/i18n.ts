// The film's words: its own catalog (messages/, i18n/vite.ts as virtual:film-messages/<locale>), which neither the
// landing nor the docs load.
import type { Catalog } from '../../../i18n/format.ts';
import { catalogReader } from '../i18n/catalog.ts';
import type { Locale } from '../i18n/locale.ts';
import type { MessageParams } from './keys.ts';
import { OTHERS } from 'virtual:film-messages';

let current: Catalog = {};
const reader = catalogReader<MessageParams>((locale, key) => {
  throw new Error(`${locale}: no message ${key}`);
});
export const { locale, t, has, say } = reader;

/** Sets the film's language and its catalog, before it mounts. */
export function use(locale: Locale, words: Catalog): void {
  current = words;
  reader.use(locale, words);
}

/** The film's catalog: every word it shows. */
export const catalog = (): Catalog => current;

/** Each language's catalog, one file each. */
export const CATALOGS: Record<Locale, () => Promise<{ default: Catalog }>> = {
  en: () => import('virtual:film-messages/en'),
  ...OTHERS,
};
