// The messages a package's build loads (client/, site/): each language as one module, `virtual:messages/<locale>`, its
// catalog with every area's file merged (`area.name` keys), so a page loads its language in one file. Over HTTP/1.1 a
// file per area, past the browser's few connections, cost new connections' round trips before the page could show
// (some 850 ms at 215 ms away).
import { resolve } from 'node:path';
import type { Plugin } from 'vite';
import { readCatalog } from './catalog.ts';

/** The plugin serving `<prefix><locale>` (virtual:messages/ by default) from `root`, a package's messages/; a package
 * with two catalogs (the site's landing and docs) runs one plugin each, each with its prefix and root. */
export function messages(root: string, prefix = 'virtual:messages/'): Plugin {
  const LANGUAGE = prefix;
  const base = resolve(root);
  const own = (path: string) => resolve(path).startsWith(`${base}/`);
  return {
    name: `messages ${prefix}`,
    resolveId: (id) => (id.startsWith(LANGUAGE) ? `\0${id}` : undefined),
    // A file added or removed changes its language's module too (an edited one is watched by `load`).
    configureServer(server) {
      const changed = (path: string) => {
        if (!own(path)) return;
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
        read = readCatalog(base, id.slice(LANGUAGE.length + 1));
      } catch (e) {
        return this.error((e as Error).message);
      }
      for (const file of read.files) this.addWatchFile(file);
      return `export default JSON.parse(${JSON.stringify(JSON.stringify(read.messages))});`;
    },
  };
}
