// The design preview: the admin page's dev server with a mock backend (mock.ts) answering from example data, for seeing
// every screen with hot reload and no stack. Run from a client directory, which may be another checkout's (see
// client/preview/run.sh): `vite --configLoader native --config <this file>`. It takes that client's own vite.config.ts and
// adds the mock; the client's build uses its vite.config.ts alone, which never loads anything here.
//
// Loaded by Node itself (`--configLoader native`), so nothing here imports a package: the client's configuration brings
// its own plugins, resolved from its own node_modules.
import { resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
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
    // The client's files, and this directory's (the page's preview runtime), which may lie outside it.
    fs: { allow: [client, fileURLToPath(new URL('.', import.meta.url))] },
  },
};
