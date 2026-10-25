// Screenshots the example screens (travel.html, signin.html) to PNGs at the display's default size, 1280x800, for
// the preview's mock stream. Run it again after changing a page, with Playwright (see CONTRIBUTING.md, "Design
// preview"), from client/preview, with a directory holding the `playwright` package's node_modules mounted beside it:
//   docker run --rm --network none --user "$(id -u):$(id -g)" -e HOME=/tmp -v "$PWD:/w/preview" \
//     -v "$PLAYWRIGHT_MODULES:/w/node_modules:ro" mcr.microsoft.com/playwright:v1.61.1-noble node /w/preview/frames/render.mjs
import { chromium } from 'playwright';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
for (const name of ['travel', 'signin']) {
  await page.goto(`file://${here}${name}.html`);
  await page.screenshot({ path: `${here}${name}.png` });
}
await browser.close();
