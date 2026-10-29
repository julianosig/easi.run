// The design preview's part in the page (the mock adds it to index.html in `vite --config preview/vite.config.ts` only):
// it plays the scenario's steps once, stands in for the browser's passkey prompt (WebAuthn cannot run against the
// mock's example challenges), and shows a chip back to the scenarios. It never reaches the client's build.

import type { PasskeyAnswer, Step } from './scenarios.ts';
import { toBase64url } from '../src/files.ts';
import { t } from '../src/i18n/index.svelte.ts';
import { remember, remembered } from '../src/ui/api.ts';

interface State {
  run: string;
  id: string;
  title: string;
  steps: Step[];
  passkey: PasskeyAnswer;
}

const state: State = await (await fetch('/__preview/state')).json();
const translate = t as unknown as (key: string, params?: Record<string, string>) => string;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** The element `find` returns once it shows (and is enabled), within a few seconds. */
async function shown<T extends HTMLElement>(find: () => T | null | undefined): Promise<T> {
  for (let tries = 0; tries < 80; tries++) {
    const el = find();
    if (el && el.offsetParent !== null && !(el as HTMLElement & { disabled?: boolean }).disabled) return el;
    await sleep(100);
  }
  throw new Error('a preview step found nothing to act on');
}

/** The button or link that shows the message `key` (or is named by it), in the page's language as it is now: each
 * attempt reads it again, so a language that loads meanwhile is the one looked for. */
const byMessage = (key: string, params?: Record<string, string>) => () => {
  const text = translate(key, params);
  return [...document.querySelectorAll<HTMLElement>('button, a')].find((el) => (el.textContent?.trim() === text || el.getAttribute('aria-label') === text) && el.offsetParent !== null);
};

async function play(): Promise<void> {
  for (const step of state.steps) {
    if ('wait' in step) await sleep(step.wait);
    else if ('fill' in step) {
      const input = await shown(() => document.querySelector<HTMLInputElement>(step.fill));
      input.focus();
      input.value = step.value;
      input.dispatchEvent(new Event('input', { bubbles: true }));
    } else if ('click' in step) (await shown(() => document.querySelector<HTMLElement>(step.click))).click();
    else (await shown(byMessage(step.press, step.params))).click();
    await sleep(150);
  }
}

// Steps play once per start of a scenario: a reload, or the page's own (after logging out), leaves the page as it is.
// Unremembered (no storage), they play again on a reload: harmless.
if (remembered('easi-preview-played', 'sessionStorage') !== state.run) {
  remember('easi-preview-played', state.run, 'sessionStorage');
  play().catch((e: unknown) => console.warn(`preview: ${(e as Error).message}`));
}

// The browser's passkey prompt, stood in for: the mock's challenges are examples no authenticator could sign.
const buffer = (n: number) => crypto.getRandomValues(new Uint8Array(n)).buffer;
function credential(create: boolean): unknown {
  const rawId = buffer(16);
  const id = toBase64url(rawId);
  const response = create
    ? { attestationObject: buffer(64), clientDataJSON: buffer(64), getTransports: () => ['internal'] }
    : { authenticatorData: buffer(37), clientDataJSON: buffer(64), signature: buffer(70), userHandle: buffer(16) };
  return { id, rawId, type: 'public-key', response, getClientExtensionResults: () => ({}) };
}

const css = `
:host{all:initial}
.scrim{position:fixed;inset:0;z-index:2147483646;display:grid;place-items:start center;background:rgb(0 0 0/.18);font:14px system-ui,sans-serif;color:#1f1f1f}
.box{margin-top:72px;width:400px;border-radius:12px;background:#fff;padding:22px 24px;box-shadow:0 12px 40px rgb(0 0 0/.3)}
.tag{display:inline-block;margin-bottom:12px;padding:2px 8px;border-radius:999px;background:#fff4e5;color:#8a4b00;font-size:11px;font-weight:600}
h1{margin:0 0 6px;font-size:18px;font-weight:600}p{margin:0;color:#444;line-height:1.45}
.key{display:flex;gap:12px;align-items:center;margin:16px 0;padding:12px;border:1px solid #dadce0;border-radius:8px}
.key i{width:28px;height:28px;border-radius:50%;background:#e8f0fe;display:grid;place-items:center;font-style:normal}
.row{display:flex;justify-content:flex-end;gap:8px;margin-top:6px}
button{font:inherit;font-weight:600;border-radius:999px;padding:8px 16px;cursor:pointer;border:1px solid #dadce0;background:#fff;color:#0b57d0}
button.go{background:#0b57d0;border-color:#0b57d0;color:#fff}
.more{margin-top:14px;font-size:12px;color:#666}.more button{padding:2px 8px;font-size:12px;font-weight:500}
.chip{position:fixed;left:12px;bottom:12px;z-index:2147483645;display:flex;align-items:center;gap:8px;padding:6px 8px 6px 12px;border-radius:999px;background:rgb(20 32 58/.88);color:#fff;font:12px system-ui,sans-serif;box-shadow:0 6px 18px rgb(0 0 0/.2)}
.chip a{color:#9fbaff;font-weight:600;text-decoration:none}.chip button{padding:0 6px;border:0;background:none;color:#cfd8ea;font-size:14px}`;

function layer(): ShadowRoot {
  const host = document.createElement('div');
  host.dataset.preview = '';
  document.body.append(host);
  const root = host.attachShadow({ mode: 'open' });
  root.innerHTML = `<style>${css}</style>`;
  return root;
}

function refusal(answer: 'cancel' | 'untrusted'): DOMException {
  return answer === 'untrusted'
    ? new DOMException('The operation is insecure.', 'SecurityError')
    : new DOMException('The operation either timed out or was not allowed.', 'NotAllowedError');
}

function prompt(create: boolean): Promise<unknown> {
  if (state.passkey !== 'ask') {
    const answer = state.passkey;
    return sleep(300).then(() => (answer === 'ok' ? credential(create) : Promise.reject(refusal(answer))));
  }
  return new Promise((resolve, reject) => {
    const root = layer();
    const site = location.hostname;
    const box = document.createElement('div');
    box.className = 'scrim';
    box.innerHTML = `<div class="box" role="dialog" aria-label="Passkey"><span class="tag">Preview: stands in for the browser’s prompt</span>
<h1>${create ? `Create a passkey for ${site}` : `Use your passkey for ${site}?`}</h1>
<p>${create ? 'This passkey is saved to this device. You can sign in with your fingerprint, face or screen lock.' : 'You’ll be asked for your fingerprint, face or screen lock.'}</p>
<div class="key"><i>🔑</i><div><b>admin</b><br><span style="color:#666">easi.run admin</span></div></div>
<div class="row"><button data-a="cancel">Cancel</button><button class="go" data-a="ok">Continue</button></div>
<div class="more">Or answer as a browser that <button data-a="untrusted">does not trust this certificate</button></div></div>`;
    root.append(box);
    box.addEventListener('click', (e) => {
      const answer = (e.target as HTMLElement).dataset.a;
      if (!answer) return;
      root.host.remove();
      if (answer === 'ok') resolve(credential(create));
      else reject(refusal(answer as 'cancel' | 'untrusted'));
    });
  });
}

if (navigator.credentials) {
  navigator.credentials.create = () => prompt(true) as Promise<Credential | null>;
  navigator.credentials.get = () => prompt(false) as Promise<Credential | null>;
}

// The way back to the scenarios; ✕ hides it for this tab.
if (remembered('easi-preview-chip', 'sessionStorage') !== 'off') {
  const root = layer();
  const chip = document.createElement('div');
  chip.className = 'chip';
  chip.innerHTML = `<span>Preview · <b></b></span><a href="/__preview/">Scenarios</a><button aria-label="Hide">✕</button>`;
  chip.querySelector('b')!.textContent = state.title;
  chip.querySelector('button')!.addEventListener('click', () => {
    remember('easi-preview-chip', 'off', 'sessionStorage');
    root.host.remove();
  });
  root.append(chip);
}

export {};
