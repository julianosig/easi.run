import { hydrate, mount } from 'svelte';
import App from './App.svelte';
import './app.css';

// The build renders the page into the HTML (scripts/prerender.mjs): hydrated in place, or mounted in development.
const target = document.getElementById('app')!;
(target.firstChild ? hydrate : mount)(App, { target });
