import { mount } from 'svelte';
import Docs from './Docs.svelte';
import { localeOf, SOURCE } from '../i18n/locale.ts';
import { CATALOGS, use } from './i18n.ts';
import './docs.css';

// The docs in the address's language (/ English, /<locale>/ each other), its messages loaded before they show; in
// English, said so, when that language's cannot load.
const language = localeOf(location.pathname);
let shown = language;
try {
  use(language, (await CATALOGS[language]()).default);
} catch {
  shown = SOURCE;
  use(SOURCE, (await CATALOGS[SOURCE]()).default);
}
document.documentElement.lang = shown;
mount(Docs, { target: document.getElementById('app')!, props: { fellBack: shown !== language } });
