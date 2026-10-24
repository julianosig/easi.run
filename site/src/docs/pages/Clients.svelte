<script lang="ts">
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';

  const features: [string, string][] = [
    ['keyboard_pointer', 'Pointer, wheel and keys from the person holding control.'],
    ['clipboard', 'Text the page copies, to the holder, and text the holder pastes.'],
    ['uploads', 'Files the holder drops on the page or gives its file chooser. Off, the chooser is answered with nothing.'],
    ['downloads', 'Files the page downloads, to the holder. Off, each one is cancelled.'],
    ['sound', 'The page’s sound, live and in recordings.'],
    ['recording', 'Recordings by the holder. A recording is a download, so it needs downloads too.'],
  ];

  const create = `POST /api/sessions
{
  "url": "https://example.com/",
  "features": { "downloads": false, "clipboard": false }
}`;

  const client = `<!-- index.html, in the directory ADMIN_STATIC points at -->
<div id="screen"></div>
<button id="take">Take control</button>
<script type="module" src="/client.js"><\/script>`;

  const script = `// client.js: log in, start a session with what you allow, show it, take control.
import { LeaseHolder, mountViewer, viewerToken } from '/sdk/viewer.js';

let csrf = '';
async function api(method, path, body) {
  const response = await fetch(path, {
    method,
    headers: { 'content-type': 'application/json', 'x-csrf-token': csrf },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) throw new Error(await response.text());
  return response.json();
}

csrf = (await api('POST', '/api/login', { password: prompt('Password') })).csrf;
const session = await api('POST', '/api/sessions', {
  url: 'https://example.com/',
  features: { downloads: false, recording: false },
});

const token = viewerToken();
const viewer = mountViewer(document.getElementById('screen'), {
  url: \`wss://\${location.host}/ws/sessions/\${session.id}?viewer=\${token}\`,
  session: session.id,
  features: session.features,
  onLease: (lease) => viewer.setControl(lease.holder === LeaseHolder.You),
  onError: (error, fatal) => fatal && console.error(error),
});
document.getElementById('take').onclick = () =>
  api('POST', \`/api/sessions/\${session.id}/control\`, { viewer: token });`;

  const options: [string, string][] = [
    ['url, session', 'The session’s viewer WebSocket and its id.'],
    ['features', 'The session’s features, as the API lists them. What is off is never sent or offered.'],
    ['onLease', 'Who holds control: you, another viewer or the agent. Call setControl when it is you.'],
    ['onCopied', 'Text the page copied, already on the clipboard or waiting for writeCopied() on a click.'],
    ['onFileRequest', 'The page opened its file chooser: call choose() from a click, or cancel().'],
    ['onUploads, onTransfer', 'Files on their way to the page, and each batch’s progress.'],
    ['onDownloads', 'Files the page downloaded; each arrives through the browser’s own download.'],
    ['onRecording', 'Where a recording is: started, ended or failed.'],
    ['onSound, onStats, onError', 'Whether sound plays, live statistics, and errors (fatal ones closed the stream).'],
  ];
  const methods: [string, string][] = [
    ['setControl(on)', 'Sends input while you hold control (off: you only watch).'],
    ['writeCopied(), setAutoCopy(on)', 'Copies the offered text, or copies everything automatically.'],
    ['setMuted(muted), setVolume(0–1)', 'Sound here only.'],
    ['startRecording({layout: wire(DEFAULT_LAYOUT), …}), stopRecording(id)', 'A recording on the server, with Capture for camera and microphone.'],
    ['close()', 'Ends the stream and removes the screen.'],
  ];
</script>

<h1 tabindex="-1">Custom clients</h1>
<p class="lead">
  Build your own interface on the same API and viewer my web client uses, and turn off, per session, what your people
  should not do.
</p>

<h2>Features per session</h2>
<p>
  Name the ones to turn off when you start a session; the rest stay on. The admin enforces them for every viewer,
  whatever the client sends, and lists them with the session.
</p>
<Code lang="http" code={create} />
<table>
  <thead><tr><th>Feature</th><th>What it covers</th></tr></thead>
  <tbody>
    {#each features as [name, what] (name)}
      <tr><td><code>{name}</code></td><td>{what}</td></tr>
    {/each}
  </tbody>
</table>
<Note>
  Features apply to every viewer of the session, and are fixed for the life of the session.
</Note>

<h2>Host your client</h2>
<ul>
  <li>Build your interface as static files and point <code>ADMIN_STATIC</code> at them, in place of my web client.</li>
  <li>It is then served from the admin’s own origin: the same login, cookie and CSRF token, and no new way in.</li>
  <li>Scripts load from files on that origin (<code>script-src 'self'</code>): no inline scripts.</li>
  <li>
    The viewer SDK is at <code>/sdk/viewer.js</code> (an ES module), with its types at
    <code>/sdk/viewer.d.ts</code>, whatever <code>ADMIN_STATIC</code> holds.
  </li>
</ul>
<Code lang="html" code={client} />
<Code lang="javascript" code={script} />

<h2>The viewer</h2>
<p>
  <code>mountViewer(element, options)</code> draws the session’s screen into the element and sends the person’s input.
  Everything around it, the bars, menus and prompts, is yours.
</p>
<table>
  <thead><tr><th>Option</th><th>What it does</th></tr></thead>
  <tbody>
    {#each options as [name, what] (name)}
      <tr><td><code>{name}</code></td><td>{what}</td></tr>
    {/each}
  </tbody>
</table>
<table>
  <thead><tr><th>Method</th><th>What it does</th></tr></thead>
  <tbody>
    {#each methods as [name, what] (name)}
      <tr><td><code>{name}</code></td><td>{what}</td></tr>
    {/each}
  </tbody>
</table>
<p>The rest of the API, sessions, control and downloads, is on the <a href="#/api">HTTP API</a> page.</p>
