// Renders the page the text-scroll fixture scrolls through and saves it as a 1280x4800 screenshot:
//   node make-text-scroll.mjs /tmp/page.png && python3 reference.py normalize /tmp/page.png ../fixtures/text-scroll.png
// The committed image is the fixture; regenerating it changes the fixture (and its references) for both systems.
import { chromium } from 'playwright';

const out = process.argv[2];
const words = 'remote screens stream pixels from a browser to a person who watches or takes control while an agent works'.split(' ');
let html = '<!doctype html><html><head><style>body{margin:0;padding:40px 0;background:#fff}'
  + 'main{max-width:760px;margin:0 auto;font:17px/1.6 Georgia,serif;color:#222}'
  + 'h2{color:#1a4f9c;font:bold 26px sans-serif;margin:1.6em 0 .4em}figure{margin:1em 0;height:180px;border-radius:8px}'
  + '</style></head><body><main>';
for (let s = 0; s < 14; s++) {
  html += `<h2>Section ${s + 1}: what the screen shows</h2>`;
  for (let p = 0; p < 3; p++) {
    html += '<p>' + Array.from({ length: 70 }, (_, i) => words[(s * 7 + p * 13 + i * 5) % words.length]).join(' ') + '.</p>';
  }
  html += `<figure style="background:linear-gradient(${s * 37}deg,hsl(${s * 50},70%,55%),hsl(${s * 50 + 120},65%,45%))"></figure>`;
}
html += '</main></body></html>';
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
await page.setContent(html);
await page.screenshot({ path: out, clip: { x: 0, y: 0, width: 1280, height: 4800 }, fullPage: true });
await browser.close();
