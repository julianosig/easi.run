// A package's catalog of one language, as the build (vite.ts, the site's prerender) and the translations' sync
// (sync.mjs) read it.
import { readdirSync, readFileSync } from 'node:fs';

/** Every message of `locale` under `root` (`<root>/<locale>/<area>.json`), by `area.name`; and the files read. */
export function readCatalog(root: string, locale: string): { messages: Record<string, unknown>; files: string[] } {
  if (!/^[a-z]{2}(-[A-Z]{2})?$/.test(locale)) throw new Error(`not a language: ${locale}`);
  const messages: Record<string, unknown> = {};
  const files: string[] = [];
  for (const file of readdirSync(`${root}/${locale}`).filter((f) => f.endsWith('.json')).sort()) {
    const path = `${root}/${locale}/${file}`;
    files.push(path);
    for (const [name, message] of Object.entries(JSON.parse(readFileSync(path, 'utf8')) as Record<string, unknown>)) {
      messages[`${file.slice(0, -'.json'.length)}.${name}`] = message;
    }
  }
  return { messages, files };
}
