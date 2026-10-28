import './app.css';
import { mount } from 'svelte';
import { initLocale } from './i18n/index.svelte.ts';
import App from './ui/App.svelte';

// The page starts in its language: nothing is drawn in English first.
await initLocale();
mount(App, { target: document.getElementById('app')! });
