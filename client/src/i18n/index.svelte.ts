// The page's language, reactively: `t(key, params)` reads the current catalog, so every template and `$derived` that
// calls it follows a switch. English is in the bundle; each other language is loaded once, when it is first chosen.
import { type ArgsOf, type Catalog, format, type Params } from '../../../i18n/format.ts';
import type { MessageParams } from './keys.ts';
import { isLocale, type Locale, match, SOURCE } from '../../../i18n/locales.ts';
import { remember as store, remembered } from '../ui/api.ts';
// English, in the bundle (vite.config.ts, `messages`).
import english from 'virtual:messages/en';
import { OTHERS } from 'virtual:messages';

/** Each other language's catalog, once loaded. */
const loaded = new Map<Locale, Catalog>([[SOURCE, english]]);
async function catalogOf(locale: Locale): Promise<Catalog> {
  const known = loaded.get(locale);
  if (known) return known;
  const catalog = locale === 'en' ? english : (await OTHERS[locale]()).default;
  loaded.set(locale, catalog);
  return catalog;
}

const STORED = 'easi-run:locale';
const state = $state<{ locale: Locale; catalog: Catalog; chosen: Locale | null }>({ locale: SOURCE, catalog: english, chosen: null });

/** The current language, and the one the person chose (null: their browser's). */
export const i18n = {
  get locale(): Locale {
    return state.locale;
  },
  get chosen(): Locale | null {
    return state.chosen;
  },
};

/** The message `key` in the current language (English where a translation is missing), its parameters filled. */
export function t<K extends keyof MessageParams>(key: K, ...[params]: ArgsOf<MessageParams, K>): string {
  // Own keys only: a name that is no key (a demo's, say) never reads an object's inherited property.
  const message = Object.hasOwn(state.catalog, key) ? state.catalog[key] : Object.hasOwn(english, key) ? english[key] : undefined;
  return message === undefined ? key : format(message, (params ?? {}) as Params, state.locale);
}

/** A text that follows the language: drawn again each time it shows, a message as `() => t(key)`, words the server
 * sent as they came. A page keeps one of these, never a finished string, where a language switch must reach it. */
export type Said = () => string;

let turn = 0;
/** Switches to `choice`, or to the browser's language with `null`, and remembers it on this browser. The current
 * language stays until the new one is loaded; of switches made in a row, the last one wins. */
export async function setLocale(choice: Locale | null, remember = true): Promise<void> {
  const mine = ++turn;
  const locale = choice ?? match(navigator.languages ?? []);
  const catalog = await catalogOf(locale);
  if (mine !== turn) return;
  state.locale = locale;
  state.catalog = catalog;
  state.chosen = choice;
  document.documentElement.lang = locale;
  // Without storage (a private window, say), the choice holds for this page only.
  if (remember) store(STORED, choice);
}

/** The language a page starts in: the one its address names (`?locale=`, which a page framing the demo gives as its
 * own, so the demo speaks it), else the one chosen on this browser, else the browser's; English if it cannot load.
 * The address's holds for that page alone: it is never remembered, so it changes no choice made on this browser. */
export async function initLocale(): Promise<void> {
  const asked = new URLSearchParams(location.search).get('locale');
  const stored = remembered(STORED);
  await setLocale(isLocale(asked) ? asked : isLocale(stored) ? stored : null, false).catch(() => undefined);
}
