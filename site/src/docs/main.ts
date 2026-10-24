import { mount } from 'svelte';
import Docs from './Docs.svelte';
import './docs.css';

mount(Docs, { target: document.getElementById('app')! });
