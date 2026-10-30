import { hydrate, mount } from 'svelte';
import App from './App.svelte';
import { CATALOGS, localeOf, use } from './i18n/index.ts';
import { pageOf } from './lib/site.ts';
import './app.css';

// The build renders each page in each language into its HTML (scripts/prerender.mjs), its language in <html lang>:
// hydrated in place, with that language's messages, or mounted in development (the language from the address). The
// page is the address's either way. A page whose messages cannot load stays as it was rendered.
const target = document.getElementById('app')!;
const rendered = target.firstChild !== null;
const language = localeOf(rendered ? `/${document.documentElement.lang}/` : location.pathname);
try {
  use(language, (await CATALOGS[language]()).default);
  (rendered ? hydrate : mount)(App, { target, props: { page: pageOf(location.pathname) } });
} catch (e) {
  console.warn('easi.run: the page stays as rendered', e);
}
