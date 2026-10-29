// A page's catalog and how it reads it, for the landing and the docs alike: each makes its own reader, so neither sees
// the other's messages. `missing` says what a key the catalog lacks reads as.
import { type ArgsOf, type Catalog, format, type Message, type Params } from '../../../i18n/format.ts';
import { type Locale, SOURCE } from '../../../i18n/locales.ts';

export function catalogReader<MessageParams>(missing: (locale: Locale, key: string) => string) {
  let current: { locale: Locale; catalog: Catalog } = { locale: SOURCE, catalog: {} };
  /** The message `key` as written, when the catalog has it. */
  const message = (key: string): Message | undefined => (Object.hasOwn(current.catalog, key) ? current.catalog[key] : undefined);
  /** `t` for a key made from data. */
  const say = (key: string, params?: Params): string => {
    const found = message(key);
    return found === undefined ? missing(current.locale, key) : format(found, params ?? {}, current.locale);
  };
  return {
    /** Sets the page's language and its catalog, before it renders. */
    use(locale: Locale, catalog: Catalog): void {
      current = { locale, catalog };
    },
    /** The page's language. */
    locale: (): Locale => current.locale,
    message,
    /** Whether the page's catalog has `key`. */
    has: (key: string): boolean => message(key) !== undefined,
    say,
    /** The message `key` in the page's language, its parameters filled. */
    t<K extends keyof MessageParams>(key: K, ...[params]: ArgsOf<MessageParams, K>): string {
      return say(key as string, params as Params | undefined);
    },
  };
}
