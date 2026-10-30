<script lang="ts">
  import Rich from '../Rich.svelte';
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';

  const features: [string, string][] = [
    ['keyboard_pointer', 'clients.feature.keyboard-pointer'],
    ['clipboard', 'clients.feature.clipboard'],
    ['uploads', 'clients.feature.uploads'],
    ['downloads', 'clients.feature.downloads'],
    ['sound', 'clients.feature.sound'],
    ['recording', 'clients.feature.recording'],
  ];

  const create = `POST /api/sessions HTTP/1.1
Content-Type: application/json

{
  "url": "https://example.com/",
  "features": { "downloads": false, "clipboard": false }
}`;

  const mount = `# compose.override.yaml, next to compose.yaml: ~/easi.run/my-client is your client
services:
  admin:
    volumes:
      - ./my-client:/srv/my-client:ro
    environment:
      ADMIN_STATIC: /srv/my-client`;

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

let login = await api('POST', '/api/login', { password: prompt('Password') });
// With the second step on, the password asks for a code: the authenticator app's, or a recovery code.
if (login.second_step) login = await api('POST', '/api/login/code', { step: login.second_step.step, code: prompt('Code') });
csrf = login.csrf;
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
    ['url, session', 'clients.option.url-session'],
    ['features', 'clients.option.features'],
    ['onLease', 'clients.option.onlease'],
    ['onCopied', 'clients.option.oncopied'],
    ['onFileRequest', 'clients.option.onfilerequest'],
    ['onUploads, onTransfer', 'clients.option.onuploads-ontransfer'],
    ['onDownloads', 'clients.option.ondownloads'],
    ['onRecording', 'clients.option.onrecording'],
    ['onSound, onStats, onError', 'clients.option.onsound-onstats-onerror'],
  ];
  const methods: [string, string][] = [
    ['setControl(on)', 'clients.method.setcontrol-on'],
    ['writeCopied(), setAutoCopy(on)', 'clients.method.writecopied-setautocopy-on'],
    ['setMuted(muted), setVolume(0–1)', 'clients.method.setmuted-muted-setvolume-0-1'],
    ['startRecording({layout: wire(DEFAULT_LAYOUT), …}), stopRecording(id)', 'clients.method.startrecording-layout-wire-default-layout-stoprecording-id'],
    ['close()', 'clients.method.close'],
  ];
</script>

<h1 tabindex="-1"><Rich message="clients.title" /></h1>
<p class="lead"><Rich message="clients.intro.1" /></p>

<h2><Rich message="clients.features-per-session.title" /></h2>
<p><Rich message="clients.features-per-session.1" /></p>
<Code lang="http" code={create} />
<table>
  <thead><tr><th><Rich message="clients.features-per-session.2" /></th><th><Rich message="clients.features-per-session.3" /></th></tr></thead>
  <tbody>
    {#each features as [name, what] (name)}
      <tr><td><code>{name}</code></td><td><Rich message={what} /></td></tr>
    {/each}
  </tbody>
</table>
<Note><Rich message="clients.features-per-session.5" /></Note>

<h2><Rich message="clients.host-your-client.title" /></h2>
<ul>
  <li><Rich message="clients.host-your-client.1" /></li>
  <li><Rich message="clients.host-your-client.2" /></li>
  <li><Rich message="clients.host-your-client.3" /></li>
  <li><Rich message="clients.host-your-client.4" /></li>
  <li><Rich message="clients.host-your-client.5" /></li>
  <li><Rich message="clients.host-your-client.6" /></li>
</ul>
<Code lang="yaml" code={mount} />
<Code lang="html" code={client} />
<Code lang="javascript" code={script} />

<h2><Rich message="clients.the-viewer.title" /></h2>
<p><Rich message="clients.the-viewer.1" /></p>
<table>
  <thead><tr><th><Rich message="clients.the-viewer.2" /></th><th><Rich message="clients.the-viewer.3" /></th></tr></thead>
  <tbody>
    {#each options as [name, what] (name)}
      <tr><td><code>{name}</code></td><td><Rich message={what} /></td></tr>
    {/each}
  </tbody>
</table>
<table>
  <thead><tr><th><Rich message="clients.the-viewer.5" /></th><th><Rich message="clients.the-viewer.6" /></th></tr></thead>
  <tbody>
    {#each methods as [name, what] (name)}
      <tr><td><code>{name}</code></td><td><Rich message={what} /></td></tr>
    {/each}
  </tbody>
</table>
<p><Rich message="clients.the-viewer.8" /></p>
