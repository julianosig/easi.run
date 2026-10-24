<script lang="ts">
  import Note from '../Note.svelte';
  import { REPO } from '../../lib/site.ts';
</script>

<h1 tabindex="-1">Security</h1>
<p class="lead">
  Every part of easi.run runs in a hardened Docker container, each with only what it needs, and every boundary is
  checked on both sides.
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
  <li>The host’s own public address is public too, so a page could reach a host service through it. The host must drop what session bridges send it: <code>iptables -I INPUT -i er-+ -j DROP</code>, written for Docker’s default userland proxy and kept across reboots (see <a href="#/quick-start">Quick start</a>).</li>
  <li>With an egress, every connection to a page and every lookup of a page’s name goes through the upstream proxy; only the upstream’s own name is resolved locally.</li>
</ul>

<h2>The admin</h2>
<ul>
  <li>The Compose file publishes its one HTTPS port on the host’s <code>127.0.0.1</code> only (inside its container it listens on <code>0.0.0.0:9443</code>).</li>
  <li>The password is kept as an Argon2id hash; repeated wrong passwords slow further attempts.</li>
  <li>Logging in and enrolling need the exact origin; every change after login needs it and a CSRF token too. The login cookie is <code>Secure</code>, <code>HttpOnly</code> and <code>SameSite=Strict</code>.</li>
  <li>An agent token reaches only the agent API and the MCP endpoint, and through them only the sessions it started. The admin keeps its SHA-256, never the token; a revoked token is refused at once and its sessions are stopped. A token carries 256 random bits, so a wrong one is answered <code>401</code> with no rate limit: it cannot be guessed, and failures never slow your login.</li>
</ul>

<h2>Control and redaction</h2>
<ul>
  <li>Input comes only from the viewer holding the control lease at the current epoch; the admin and the display each check it.</li>
  <li>Taking control stops the agent first, and proves it stopped.</li>
  <li>After a takeover, text the person typed or pasted (4+ characters, unchanged) is replaced by <code>[redacted]</code> in later agent results, and screenshots are no longer returned.</li>
</ul>

<h2>What is kept</h2>
<ul>
  <li>A session’s browser profile, downloads and uploads live in memory inside its containers, and are removed when the session stops.</li>
  <li>A recording is written only to the download stream of the viewer who started it: the server keeps no copy. The camera and microphone it receives are used for that recording alone.</li>
  <li>The admin keeps its password hash, its agent tokens’ hashes, and its TLS certificate and private key in its state volume.</li>
  <li>Each session’s proxy logs the destinations it connects to; Docker keeps up to about 30 MB of each container’s logs on disk (rotated: older lines go first), until the session’s containers are removed.</li>
</ul>

<Note tone="warn">
  <strong>The Docker socket is the host.</strong> Whoever controls Docker controls the server. The controller is the
  only container that holds it; it has no network, serves only the admin, and creates only its fixed containers. Keep
  the host and Docker up to date.
</Note>

<p>
  The full threat model, with the test that proves each boundary, is in
  <a href="{REPO}/blob/main/docs/security.md">docs/security.md</a>. To report a vulnerability, follow
  <a href="{REPO}/blob/main/SECURITY.md">SECURITY.md</a>.
</p>
