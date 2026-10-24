// Builds the benchmark page into dist/ (after `vite build` made the admin page there), or (with --sdk) the viewer SDK
// into dist/sdk/, or (with --tests) the unit tests into .test-out/.
import { build } from 'esbuild';
import { execFileSync } from 'node:child_process';
import { copyFile, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';

const root = new URL('..', import.meta.url).pathname;

if (process.argv.includes('--sdk')) {
  // The viewer SDK, added to the admin page's dist/ that `vite build` made first: one ESM file, and its types.
  await rm(`${root}dist/sdk`, { recursive: true, force: true });
  await build({
    absWorkingDir: root,
    entryPoints: { viewer: 'src/sdk.ts' },
    outdir: 'dist/sdk',
    bundle: true,
    platform: 'browser',
    format: 'esm',
    target: 'es2022',
    minify: true,
    legalComments: 'none',
    logLevel: 'warning',
  });
  execFileSync('npx', ['--no-install', 'tsc', '-p', 'tsconfig.sdk.json'], { cwd: root, stdio: 'inherit' });
  // The declarations import each other as the sources do (`./protocol.ts`); as published they are `.js` modules.
  for (const name of await readdir(`${root}dist/sdk/types`)) {
    const path = `${root}dist/sdk/types/${name}`;
    await writeFile(path, (await readFile(path, 'utf8')).replace(/(from '\.\/[\w-]+)\.ts'/g, "$1.js'"));
  }
  // The types' entry sits next to the module it types.
  await writeFile(`${root}dist/sdk/viewer.d.ts`, "export * from './types/sdk.js';\n");
} else if (process.argv.includes('--tests')) {
  const tests = (await readdir(`${root}test`)).filter((name) => name.endsWith('.test.ts'));
  await rm(`${root}.test-out`, { recursive: true, force: true });
  await build({
    absWorkingDir: root,
    entryPoints: tests.map((name) => `test/${name}`),
    outdir: '.test-out',
    outExtension: { '.js': '.mjs' },
    bundle: true,
    platform: 'node',
    format: 'esm',
    target: 'node26',
    loader: { '.txt': 'text' },
    logLevel: 'warning',
  });
} else {
  // The benchmark page, added to the admin page's dist/ that `vite build` made first (the bench profile only).
  await build({
    absWorkingDir: root,
    entryPoints: { bench: 'src/bench.ts' },
    outdir: 'dist',
    bundle: true,
    platform: 'browser',
    format: 'esm',
    target: 'es2022',
    minify: true,
    legalComments: 'none',
    logLevel: 'warning',
  });
  await mkdir(`${root}dist`, { recursive: true });
  await copyFile(`${root}bench/index.html`, `${root}dist/bench.html`);
  await copyFile(`${root}bench/bench.css`, `${root}dist/bench.css`);
}
