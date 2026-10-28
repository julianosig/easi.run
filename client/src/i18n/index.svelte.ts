// The page's language, reactively: `t(key, params)` reads the current catalog, so every template and `$derived` that
// calls it follows a switch. English is in the bundle; each other language is loaded once, when it is first chosen.
import { type Catalog, format, type Message, type Params } from '../../../i18n/format.ts';
import type { MessageParams } from './keys.ts';
import { isLocale, type Locale, match, SOURCE } from '../../../i18n/locales.ts';

/** One catalog from its area files (`messages/<locale>/<area>.json`, each key `name` becoming `area.name`). */
function merge(files: Record<string, Record<string, Message>>): Catalog {
  const catalog: Catalog = {};
  for (const [path, entries] of Object.entries(files)) {
    const area = path.slice(path.lastIndexOf('/') + 1, -'.json'.length);
    for (const [name, message] of Object.entries(entries)) catalog[`${area}.${name}`] = message;
  }
  return catalog;
}

const english = merge(import.meta.glob<Record<string, Message>>('../../messages/en/*.json', { eager: true, import: 'default' }));
/** Each other language, as one file (vite.config.ts, `messages`). */
const others: Record<Exclude<Locale, 'en'>, () => Promise<{ default: Catalog }>> = {
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

/** Each other language's catalog, once loaded. */
const loaded = new Map<Locale, Catalog>([[SOURCE, english]]);
async function catalogOf(locale: Locale): Promise<Catalog> {
  const known = loaded.get(locale);
  if (known) return known;
  const catalog = locale === 'en' ? english : (await others[locale]()).default;
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

type Args<K extends keyof MessageParams> = MessageParams[K] extends Record<string, never> ? [] : [MessageParams[K]];

/** The message `key` in the current language (English where a translation is missing), its parameters filled. */
export function t<K extends keyof MessageParams>(key: K, ...[params]: Args<K>): string {
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
  if (!remember) return;
  try {
    if (choice === null) localStorage.removeItem(STORED);
    else localStorage.setItem(STORED, choice);
  } catch {
    // No storage (a private window, say): the choice holds for this page only.
  }
}

/** The language a page starts in: the one chosen on this browser, else the browser's; English if it cannot load. */
export async function initLocale(): Promise<void> {
  let stored: string | null = null;
  try {
    stored = localStorage.getItem(STORED);
  } catch {
    // No storage: the browser's language.
  }
  await setLocale(isLocale(stored) ? stored : null, false).catch(() => undefined);
}
