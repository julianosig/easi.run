import { mount } from 'svelte';
import Film from './Film.svelte';
import { CATALOGS, use } from './i18n.ts';
import type { Locale } from '../i18n/index.ts';
import '../app.css';

// The film's stage (see scripts/film.mjs): `?locale=` its language.
const locale = (new URLSearchParams(location.search).get('locale') ?? 'en') as Locale;
use(locale, (await CATALOGS[locale]()).default);
document.documentElement.lang = locale;
mount(Film, { target: document.getElementById('app')! });
