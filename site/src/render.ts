// The build's entry for rendering one page in one language (scripts/prerender.mjs).
import { render } from 'svelte/server';
import type { Catalog } from '../../i18n/format.ts';
import App from './App.svelte';
import { type Locale, use } from './i18n/index.ts';
import type { Page } from './lib/site.ts';

export function page(locale: Locale, catalog: Catalog, which: Page): { head: string; body: string } {
  use(locale, catalog);
  return render(App, { props: { page: which } });
}
