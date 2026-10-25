<script lang="ts">
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  import { REPO } from '../../lib/site.ts';
  const prompt = 'Read https://easi.run/install.md and install easi.run on this server.';
  const install = 'curl -fsSLO https://easi.run/install.sh && less install.sh && bash install.sh';
  const tunnel = 'ssh -N -L 9443:127.0.0.1:9443 you@your-server';
  const run = `git clone ${REPO} && cd easi.run
DOCKER_GID=$(stat -c %g /var/run/docker.sock) docker compose up -d --build
docker compose exec admin admin setup`;
  const firewall = 'sudo iptables -t raw -I PREROUTING 1 -i er-+ -m addrtype --dst-type LOCAL -j DROP';
</script>

<h1 tabindex="-1">Quick start</h1>
<p class="lead">
  From a fresh Linux server to a watched browser session in about 15 minutes on 2 cores, most of it the first build.
</p>

<h2>1. Install it</h2>
<p>
  You need Linux on x86-64 with systemd and Docker Engine 28 or newer, with Compose. Ask your coding agent on the
  server:
</p>
<Code lang="prompt" code={prompt} />
<p>Or run the installer yourself, after reading it:</p>
<Code lang="shell" code={install} />
<p>It runs as your own account and, in order:</p>
<ol>
  <li>checks the server: Docker, memory, disk and port 9443;</li>
  <li>clones easi.run into <code>~/easi.run</code> at its latest release (a pinned commit until the first);</li>
  <li>
    puts the firewall rule that keeps pages off the server’s own services in place, and makes Docker put it back
    before every start, so Docker and its containers start only once it is there;
  </li>
  <li>builds the images, with Google’s current Chrome stable from Google’s signed repository;</li>
  <li>starts easi.run, and checks it over HTTPS against the admin’s own certificate;</li>
  <li>
    offers easi.run to Claude Code over MCP (see <a href="#/agents">Agents</a>);
  </li>
  <li>
    prints a summary: the setup link where you create the admin’s password, whether MCP is ready, and the lines for
    another computer.
  </li>
</ol>
<p>
  <code>bash install.sh --dry-run</code> runs the checks and changes nothing; <code>bash install.sh --update</code>
  moves an installation to a newer release.
</p>

<h2>2. Open it</h2>
<p>
  The admin is at <code>https://localhost:9443</code>, on the server’s <code>127.0.0.1</code> only. From your
  computer, reach it through an SSH tunnel:
</p>
<Code lang="shell" code={tunnel} />
<p>
  The certificate is self-signed: the installer keeps a copy in <code>~/.config/easi-run/tls-cert.pem</code>, and
  the admin logs its SHA-256 fingerprint at start, so you can check it before you accept it.
</p>

<h2>3. Create your password and start a session</h2>
<ol>
  <li>
    Open the setup link the summary printed (from your computer, through the tunnel) and choose your password, 12
    characters or more; you are logged in at once. The link works once, for up to 15 minutes; to print it again, run
    <code>docker exec -i easi-run-admin-1 admin setup</code> on the server. From then on, log in with your password.
  </li>
  <li>Type a URL and press <strong>Start session</strong>, or ask your agent to open one.</li>
  <li>Watch the page, or press <strong>Take control</strong> to use it yourself, then <strong>Give back</strong>.</li>
</ol>

<h2>Without the installer</h2>
<p>
  First, the firewall rule. Pages reach only public addresses, and this rule keeps them off the services on every
  address the host holds, the ports Docker publishes on them included: it drops whatever the session bridges
  (<code>er-&lt;session&gt;</code>) send one of those addresses, before Docker’s port redirects. Keep it across reboots
  with your host’s firewall configuration.
</p>
<Code lang="shell" code={firewall} />
<p>Then start easi.run:</p>
<Code lang="shell" code={run} />
<p>
  The last line prints the <strong>setup link</strong>: open it and choose your password, 12 characters or more. It
  works once, for up to 15 minutes; once it expires, the same line prints a new one. <code>DOCKER_GID</code> gives the controller
  the group of the Docker socket; without it, no session can start.
</p>

<Note>
  Each browser session gets 2 CPUs and 2048 MiB by default. The installer runs as many at once as the server’s memory
  holds (one on 4 GB, two on 8 GB, four on 16 GB, up to 16); by hand, one runs at once. Change that with <code>BROWSER_CPUS</code>, <code>BROWSER_MEMORY_MIB</code> and <code>ADMIN_MAX_SESSIONS</code>; see
  <a href="#/configuration">Configuration</a>.
</Note>
