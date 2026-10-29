// Whether the visitor asks for less motion, reactively: what moves on its own stops as soon as they do.
import { MediaQuery } from 'svelte/reactivity';

export const reducedMotion = new MediaQuery('(prefers-reduced-motion: reduce)');
