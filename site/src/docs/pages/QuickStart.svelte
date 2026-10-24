<script lang="ts">
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  import { REPO } from '../../lib/site.ts';
  const run = `git clone ${REPO} && cd easi.run
DOCKER_GID=$(stat -c %g /var/run/docker.sock) docker compose up -d --build
docker compose logs admin | grep -i enroll`;
  const firewall = 'sudo iptables -I INPUT -i er-+ -j DROP';
  const tunnel = 'ssh -L 9443:127.0.0.1:9443 you@your-server';
</script>

<h1 tabindex="-1">Quick start</h1>
<p class="lead">From a fresh Linux server to a watched browser session in a few minutes.</p>

<h2>1. Start it</h2>
<p>You need Linux on x86-64 with Docker Engine 28 or newer.</p>
<Code lang="shell" code={run} />
<p>
  The last line prints a <strong>one-time enrollment token</strong>, valid for 15 minutes; once it expires,
  <code>docker compose restart admin</code> logs a new one (the last in the log). <code>DOCKER_GID</code> gives
  the controller the group of the Docker socket; without it, no session can start.
</p>

<h2>2. Close the host to sessions</h2>
<p>
  Pages reach only public addresses. This rule keeps them off the host’s own services, its public address included:
  it drops whatever the session bridges (<code>er-&lt;session&gt;</code>) send the host itself.
</p>
<Code lang="shell" code={firewall} />
<p>
  Keep the rule across reboots with your host’s firewall configuration, and keep Docker’s default userland proxy
  (<code>"userland-proxy": true</code>, as Docker ships it): published ports then reach the host through the same
  input chain, so the rule covers them too.
</p>

<h2>3. Open it</h2>
<p>
  The admin listens on <code>https://localhost:9443</code>, on loopback only. On a remote server, reach it through an
  SSH tunnel:
</p>
<Code lang="shell" code={tunnel} />
<p>
  The certificate is self-signed. The admin logs its SHA-256 fingerprint at start, so you can check it before you
  accept it.
</p>

<h2>4. Enroll and start a session</h2>
<ol>
  <li>Enter the enrollment token and a password of at least 12 characters.</li>
  <li>Log in, type a URL and press <strong>Start session</strong>.</li>
  <li>Watch the page, or press <strong>Take control</strong> to use it yourself, then <strong>Give back</strong>.</li>
</ol>

<Note>
  Each browser session gets 2 CPUs and 2048 MiB by default. Change that with <code>BROWSER_CPUS</code> and
  <code>BROWSER_MEMORY_MIB</code>; see <a href="#/configuration">Configuration</a>.
</Note>
