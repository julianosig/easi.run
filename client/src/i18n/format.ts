// Messages as the catalogs hold them, and how one becomes text: plain TypeScript, so the unit tests run it as is.

/** One message: text with `{name}` placeholders, or a plural message, one text per CLDR category of its locale
 * (`other` always), picked by its `count` parameter. */
export type Message = string | PluralMessage;
export type PluralMessage = { other: string } & Partial<Record<Intl.LDMLPluralRule, string>>;
/** Every message of a locale, by key (`area.name`). */
export type Catalog = Record<string, Message>;
export type Params = Record<string, string | number>;

const PLACEHOLDER = /\{([a-zA-Z][a-zA-Z0-9_]*)\}/g;

/** The placeholder names of a message's every text, sorted and each once. */
export function placeholders(message: Message): string[] {
  const texts = typeof message === 'string' ? [message] : Object.values(message);
  const names = new Set<string>();
  for (const text of texts) for (const m of text.matchAll(PLACEHOLDER)) names.add(m[1]!);
  return [...names].sort();
}

const rules = new Map<string, Intl.PluralRules>();
function pluralRules(locale: string): Intl.PluralRules {
  let found = rules.get(locale);
  if (!found) rules.set(locale, (found = new Intl.PluralRules(locale)));
  return found;
}

/** `message` as text for `locale`: a plural message's form for `params.count`, its placeholders filled (numbers in the
 * locale's own digits and grouping). A placeholder with no parameter stays as written. */
export function format(message: Message, params: Params, locale: string): string {
  let text: string;
  if (typeof message === 'string') {
    text = message;
  } else {
    const count = Number(params.count ?? 0);
    text = message[pluralRules(locale).select(count)] ?? message.other;
  }
  return text.replace(PLACEHOLDER, (whole, name: string) => {
    const value = params[name];
    if (value === undefined) return whole;
    return typeof value === 'number' ? value.toLocaleString(locale) : value;
  });
}
