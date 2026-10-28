// The design preview: the admin page's dev server with a mock backend (mock.ts) answering from example data, for seeing
// every screen with hot reload and no stack. Run from a client directory, with this directory as its preview/ (see
// client/preview/run.sh): `vite --configLoader runner --config preview/vite.config.ts`. It takes that client's own
// vite.config.ts and adds the mock; the client's build uses its vite.config.ts alone, which never loads anything here.
//
// Vite's module runner loads it, so it may import the client's own TypeScript (the mock speaks its protocol).
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { preview } from './mock.ts';

const client = process.cwd();
const base = (await import(pathToFileURL(resolve(client, 'vite.config.ts')).href)).default;

export default {
  ...base,
  plugins: [...(base.plugins ?? []), preview()],
  // The page alone: the benchmark's page (bench/index.html) names a script only its own build makes.
  optimizeDeps: { ...base.optimizeDeps, entries: ['index.html'] },
  server: {
    ...base.server,
    // Loopback only; in its container (client/preview/run.sh) it listens on all of the container's addresses, behind a port
    // published on the host's loopback alone.
    host: '127.0.0.1',
    port: 5181,
    strictPort: true,
    allowedHosts: ['localhost', '127.0.0.1'],
    // The client's files, this directory (the page's preview runtime) among them, and the i18n/ it shares with the site.
    fs: { allow: [client, resolve(client, '../i18n')] },
  },
};
