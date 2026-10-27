// Builds the benchmark page into dist/ (after `vite build` made the admin page there), or (with --sdk) the viewer SDK
// into dist/sdk/, or (with --tests) the unit tests into .test-out/, or (with --compress) writes a Brotli and a gzip copy
// beside each text file of dist/, which the admin serves to a browser that accepts them.
import { build } from 'esbuild';
import { execFileSync } from 'node:child_process';
import { copyFile, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { brotliCompressSync, constants, gzipSync } from 'node:zlib';

const root = new URL('..', import.meta.url).pathname;
// The SDK's and the benchmark page's bundle: one minified ES module for the browser.
const browser = {
  absWorkingDir: root,
  bundle: true,
  platform: 'browser',
  format: 'esm',
  target: 'es2022',
  minify: true,
  legalComments: 'none',
  logLevel: 'warning',
};

if (process.argv.includes('--compress')) {
  // Fonts (woff2) and images are compressed already.
  const text = /\.(js|css|html|svg|txt|d\.ts)$/;
  for (const entry of await readdir(`${root}dist`, { recursive: true, withFileTypes: true })) {
    if (!entry.isFile() || !text.test(entry.name)) continue;
    const path = `${entry.parentPath}/${entry.name}`;
    const body = await readFile(path);
    const br = brotliCompressSync(body, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_SIZE_HINT]: body.length } });
    await writeFile(`${path}.br`, br);
    await writeFile(`${path}.gz`, gzipSync(body, { level: 9 }));
  }
} else if (process.argv.includes('--sdk')) {
  // The viewer SDK, added to the admin page's dist/ that `vite build` made first: one ESM file, and its types.
  await rm(`${root}dist/sdk`, { recursive: true, force: true });
  await build({ ...browser, entryPoints: { viewer: 'src/sdk.ts' }, outdir: 'dist/sdk' });
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
    // For a coverage run (scripts/coverage.sh): each bundle maps back to its sources, and keeps all of each module it
    // imports, so the code no test runs counts as uncovered instead of vanishing from the bundle.
    sourcemap: 'inline',
    treeShaking: false,
    logLevel: 'warning',
  });
} else {
  // The benchmark page, added to the admin page's dist/ that `vite build` made first (the bench profile only).
  await build({ ...browser, entryPoints: { bench: 'src/bench.ts' }, outdir: 'dist' });
  await copyFile(`${root}bench/index.html`, `${root}dist/bench.html`);
  await copyFile(`${root}bench/bench.css`, `${root}dist/bench.css`);
}
