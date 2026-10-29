// Each language but English, loaded when first chosen: its catalog as one module from the package's messages plugin
// (vite.ts), the same table in every package that serves `virtual:messages/`.
import type { Catalog } from './format.ts';
import type { Locale } from './locales.ts';

export const OTHERS: Record<Exclude<Locale, 'en'>, () => Promise<{ default: Catalog }>> = {
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
