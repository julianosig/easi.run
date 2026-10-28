// The languages the admin page speaks, each named in itself, and which one a browser's own list picks.

export const LOCALES = ['en', 'pt-BR', 'es', 'zh-CN', 'hi', 'fr', 'de', 'ja', 'ru', 'id', 'ko'] as const;
export type Locale = (typeof LOCALES)[number];
export const SOURCE: Locale = 'en';

/** Each language's own name for itself, as the picker lists it. */
export const AUTONYMS: Record<Locale, string> = {
  en: 'English',
  'pt-BR': 'Português (Brasil)',
  es: 'Español',
  'zh-CN': '简体中文',
  hi: 'हिन्दी',
  fr: 'Français',
  de: 'Deutsch',
  ja: '日本語',
  ru: 'Русский',
  id: 'Bahasa Indonesia',
  ko: '한국어',
};

export function isLocale(value: unknown): value is Locale {
  return typeof value === 'string' && (LOCALES as readonly string[]).includes(value);
}

/** The first of a browser's languages (`navigator.languages`, best first) the page speaks: the exact tag, else its
 * language alone (`pt-PT` speaks `pt-BR`, `zh-TW` and `zh-HK` too: the only Chinese and Portuguese there are); English
 * if none. */
export function match(languages: readonly string[]): Locale {
  for (const tag of languages) {
    const exact = LOCALES.find((l) => l.toLowerCase() === tag.toLowerCase());
    if (exact) return exact;
    const language = tag.split('-')[0]!.toLowerCase();
    const near = LOCALES.find((l) => l.split('-')[0]!.toLowerCase() === language);
    if (near) return near;
  }
  return SOURCE;
}
