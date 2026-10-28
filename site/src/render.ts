// The build's entry for rendering the page in one language (scripts/prerender.mjs).
import { render } from 'svelte/server';
import type { Catalog } from '../../i18n/format.ts';
import App from './App.svelte';
import { type Locale, use } from './i18n/index.ts';

export function page(locale: Locale, catalog: Catalog): { head: string; body: string } {
  use(locale, catalog);
  return render(App);
}
