// The messages a package's build loads (client/, site/): each language as one module, `virtual:messages/<locale>`, its
// catalog with every area's file merged (`area.name` keys), so a page loads its language in one file. Over HTTP/1.1 a
// file per area, past the browser's few connections, cost new connections' round trips before the page could show
// (some 850 ms at 215 ms away).
import { readdirSync, readFileSync } from 'node:fs';
import type { Plugin } from 'vite';

const LANGUAGE = 'virtual:messages/';

/** Every message of `locale` under `root` (`<root>/<locale>/<area>.json`), by `area.name`; and the files read. */
export function catalog(root: string, locale: string): { messages: Record<string, unknown>; files: string[] } {
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

/** The plugin serving `virtual:messages/<locale>` from `root` (a package's messages/). */
export function messages(root: string): Plugin {
  return {
    name: 'messages',
    resolveId: (id) => (id.startsWith(LANGUAGE) ? `\0${id}` : undefined),
    // A file added or removed changes its language's module too (an edited one is watched by `load`).
    configureServer(server) {
      const changed = (path: string) => {
        const locale = /messages[\\/]([^\\/]+)[\\/][^\\/]+\.json$/.exec(path)?.[1];
        const module = locale === undefined ? undefined : server.moduleGraph.getModuleById(`\0${LANGUAGE}${locale}`);
        if (!module) return;
        server.moduleGraph.invalidateModule(module);
        server.ws.send({ type: 'full-reload' });
      };
      server.watcher.on('add', changed).on('unlink', changed);
    },
    load(id) {
      if (!id.startsWith(`\0${LANGUAGE}`)) return undefined;
      let read;
      try {
        read = catalog(root, id.slice(LANGUAGE.length + 1));
      } catch (e) {
        return this.error((e as Error).message);
      }
      for (const file of read.files) this.addWatchFile(file);
      return `export default JSON.parse(${JSON.stringify(JSON.stringify(read.messages))});`;
    },
  };
}
