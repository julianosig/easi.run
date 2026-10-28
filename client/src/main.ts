import './app.css';
import { mount } from 'svelte';
import { initLocale } from './i18n/index.svelte.ts';
import App from './ui/App.svelte';

// The page's language loads while the page asks the admin what to show; no view shows before both are in, so nothing
// is drawn in English first.
mount(App, { target: document.getElementById('app')!, props: { language: initLocale() } });
