<script lang="ts">
  import Note from '../Note.svelte';
  import { REPO } from '../../lib/site.ts';
</script>

<h1 tabindex="-1">Security</h1>
<p class="lead">
  Every part of easi.run runs in a hardened Docker container, each with only what it needs. Input, the clipboard and
  downloads are checked twice, by the admin and by the display.
</p>

<h2>Containers and what each may do</h2>
<table>
  <thead><tr><th>Container</th><th>Runs</th><th>May</th></tr></thead>
  <tbody>
    <tr><td><strong>admin</strong></td><td>All the time</td><td>Serve one HTTPS port; talk to the controller and to each session’s display. No Docker access.</td></tr>
    <tr><td><strong>controller</strong></td><td>All the time</td><td>Hold the Docker socket, with no network, serving only the admin’s uid, and only a closed set of operations on fixed container templates.</td></tr>
    <tr><td><strong>browser</strong></td><td>Per session</td><td>Run Chrome, with its own sandbox, no direct network, no capabilities, no devices, a read-only root and a seccomp profile. Its only way out is the session’s proxy.</td></tr>
    <tr><td><strong>proxy</strong></td><td>Per session</td><td>Connect to public addresses on ports 80 and 443, or to the session’s upstream proxy.</td></tr>
    <tr><td><strong>display</strong></td><td>Per session</td><td>Show the browser and encode its picture and sound. It never maps the browser’s memory.</td></tr>
    <tr><td><strong>agent</strong></td><td>Per session</td><td>Read the page’s accessibility tree and send virtual input, only while no person holds control.</td></tr>
  </tbody>
</table>
<ul>
  <li>By default each role has its own uid; every container has no capabilities, a read-only root, and limits on CPU and memory, and each session’s containers on processes too.</li>
  <li>The images have no shell and no package manager: five are distroless, and the browser’s is a pruned Debian with Chrome and the libraries it links.</li>
</ul>

<h2>Network</h2>
<ul>
  <li>The browser joins no Docker network. It reaches the session’s proxy over a Unix socket.</li>
  <li>The proxy resolves names itself and connects to the address it checked, so DNS rebinding cannot redirect it. It refuses private, loopback, link-local and other special destination addresses, and every port but 80 and 443.</li>
  <li>One host firewall rule keeps pages off the services on every address the host holds: first in the raw table, before Docker’s port redirects, it drops whatever the session bridges (<code>er-&lt;session&gt;</code>) send one of those addresses, the ports Docker publishes included. The installer puts it first before every start of Docker; by hand, keep it across reboots (see <a href="#/quick-start">Quick start</a>).</li>
  <li>With an egress, every connection to a page and every lookup of a page’s name goes through the upstream proxy; only the upstream’s own name is resolved locally.</li>
</ul>

<h2>The admin</h2>
<ul>
  <li>The Compose file publishes its one HTTPS port on the host’s <code>127.0.0.1</code> only (inside its container it listens on <code>0.0.0.0:9443</code>).</li>
  <li>The password is kept as an Argon2id hash; repeated wrong passwords slow further attempts.</li>
  <li>An optional second step at sign-in (see <a href="#/quick-start">Quick start</a>): a code from an authenticator app, which signs in once, a recovery code, or a passkey with user verification. With it on, the password alone opens nothing, and wrong codes count toward the same wait as wrong passwords.</li>
  <li>The one-time setup link that sets the password carries 256 random bits, works once, expires after 15 minutes, and is never logged.</li>
  <li>Logging in and setting the password need the exact origin; every change after login needs it and a CSRF token too. The login cookie is <code>Secure</code>, <code>HttpOnly</code> and <code>SameSite=Strict</code>.</li>
  <li>An agent token reaches only the agent API and the MCP endpoint, and through them only the sessions it started. The admin keeps its SHA-256, never the token, and a revoked token is refused at once.</li>
</ul>
<p>Each of these, with the test that proves it, is in <a href="{REPO}/blob/main/docs/security.md#boundaries">docs/security.md</a> (Admin application, Agent tokens).</p>

<h2>Control and redaction</h2>
<ul>
  <li>Input comes only from the viewer holding the control lease at the current epoch; the admin and the display each check it.</li>
  <li>Taking control first pauses the agent’s calls and proves none still runs (see <a href="#/agent">The agent</a>).</li>
  <li id="redaction">After a takeover, text of four characters or more you type or paste is replaced by <code>[redacted]</code> in every later result the agent’s caller receives, matched exactly as written; your typing is followed through your edits until you leave the value (Enter, Tab, a click, Escape or a move).</li>
  <li>In the accessibility tree the driver returns, a password field’s value is replaced by <code>[redacted]</code>. From the first takeover on, every image item and every field named <code>*_b64</code>, where the driver returns screenshots, is removed from the agent’s results.</li>
</ul>

<h2>What is kept</h2>
<ul>
  <li>A session’s browser profile, downloads and uploads live in memory inside its containers, never in swap, and are removed when the session stops; a session started from a saved profile saves it first, encrypted. Every container runs with a core limit the kernel takes as no core dump at all, so a crash never writes a process’s memory to the server’s disk or hands it to a crash collector.</li>
  <li>A recording is written only to the download stream of the viewer who started it: the server keeps no copy. The camera and microphone it receives are used for that recording alone.</li>
  <li>The admin keeps its password hash, its agent tokens’ hashes, the second step (the authenticator app’s secret, the recovery codes’ hashes and the passkeys’ public keys, in a file of mode 0600), the saved profiles (each one’s name, the tokens it allows, its wrapped key and its encrypted data, in files of mode 0600), and its TLS certificate and private key in its state volume.</li>
  <li>Each session’s proxy logs the destinations it connects to; Docker keeps up to about 30 MB of each container’s logs on disk (rotated: older lines go first), until the session’s containers are removed.</li>
</ul>

<h2>Saved profiles</h2>
<ul>
  <li>Your password unlocks them: it derives a key (Argon2id) that the admin holds only in memory, never written or logged, which wraps each profile’s own random key, which encrypts its data (XChaCha20-Poly1305): a changed, reordered or cut file does not open. After a restart they stay locked until you enter your password.</li>
  <li>A profile’s data is written to no file in the clear, and the browser unpacks it as untrusted; <a href="{REPO}/blob/main/docs/security.md#boundaries">docs/security.md</a> (Saved profiles) says where it passes.</li>
  <li>Losing your password loses the profiles: they cannot be opened without it. Delete them and log in on those sites again in new ones.</li>
</ul>

<Note>
  <strong>Only the controller holds the Docker socket.</strong> It has no network, serves only the admin’s uid, and
  runs only its closed set of operations on fixed templates. Keep the host and Docker up to date.
</Note>

<p>
  The full threat model, with the test that proves each boundary, is in
  <a href="{REPO}/blob/main/docs/security.md">docs/security.md</a>. To report a vulnerability, follow
  <a href="{REPO}/blob/main/SECURITY.md">SECURITY.md</a>.
</p>
