// The demo inside a page that frames it (ADMIN_FRAME_ANCESTORS): that page shows the list of demos and picks one,
// so the demo bar leaves its own list out. The two talk by messages, each checking the other's origin:
//   demo -> page  { type: 'easi-run:demos', demos: [{ name, title, line, icon }], current, active }
//   page -> demo  { type: 'easi-run:pick', demo }
//   page -> demo  { type: 'easi-run:ask' }: the page is listening (it may have missed what the demo said before),
//                 and the demo says it again
// While no visit is under way (it ended, or has not started), `active` is false: the page then reloads the frame at
// `?demo=<name>`, and the visit that starts opens that demo; when the reload brings back a visit still under way at
// another (the connection was lost, the visit not), the page picks its demo once that visit says it is active.
// The frame's address also names the page's language (`?locale=`, see initLocale), which the demo then speaks, its
// visit's browser too: a visit under way in another language is not brought back, one in this language replaces it.

import english from '../../messages/en/demo.json';
import { type DemoText, demoOf } from './demos.ts';

/** A demo's text as the framing page reads it: English, whatever this page's language (its messages are a protocol). */
function inEnglish(key: DemoText): string {
  const name = key.slice('demo.'.length);
  const message: unknown = key.startsWith('demo.') && Object.hasOwn(english, name) ? (english as Record<string, unknown>)[name] : undefined;
  return typeof message === 'string' ? message : key;
}

/** The origin of the page framing this one, when it is one allowed to; null when not framed (or by another page). */
export function embedder(allowed: string[]): string | null {
  if (window.parent === window) return null;
  const ancestor = location.ancestorOrigins?.[0] ?? (document.referrer ? new URL(document.referrer).origin : '');
  return allowed.includes(ancestor) ? ancestor : null;
}

/** Tells the framing page the demos, which one is on, and whether a visit is under way to switch it. */
export function announce(origin: string, sites: string[], current: string, active: boolean): void {
  const demos = sites.map((name) => {
    const { title, line, icon } = demoOf(name);
    return { name, title: inEnglish(title), line: inEnglish(line), icon };
  });
  window.parent.postMessage({ type: 'easi-run:demos', demos, current, active }, origin);
}

/** The demo a framing page asked to start at (`?demo=`), when it is one of `sites`. */
export function asked(sites: string[]): string | null {
  const name = new URLSearchParams(location.search).get('demo');
  return name !== null && sites.includes(name) ? name : null;
}

/** Calls `pick` with each demo the framing page picks among `sites`, and `ask` each time it asks what is on; returns
 * the unsubscribe. */
export function picks(origin: string, sites: () => string[], pick: (name: string) => void, ask: () => void): () => void {
  const listener = (e: MessageEvent) => {
    if (e.source !== window.parent || e.origin !== origin) return;
    const data = e.data as { type?: unknown; demo?: unknown } | null;
    if (data?.type === 'easi-run:pick' && typeof data.demo === 'string' && sites().includes(data.demo)) pick(data.demo);
    if (data?.type === 'easi-run:ask') ask();
  };
  window.addEventListener('message', listener);
  return () => window.removeEventListener('message', listener);
}
