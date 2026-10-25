<script lang="ts">
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  import { REPO } from '../../lib/site.ts';
  const prompt = 'Read https://easi.run/install.md and install easi.run on this server.';
  const install = 'curl -fsSLO https://easi.run/install.sh && less install.sh && bash install.sh';
  const tunnel = 'ssh -N -L 9443:127.0.0.1:9443 you@your-server';
  const run = `git clone ${REPO} && cd easi.run
DOCKER_GID=$(stat -c %g /var/run/docker.sock) docker compose up -d --build`;
  const firewall = 'sudo iptables -t raw -I PREROUTING 1 -i er-+ -m addrtype --dst-type LOCAL -j DROP';
</script>

<h1 tabindex="-1">Quick start</h1>
<p class="lead">
  From a fresh Linux server to a watched browser session in a few minutes: nothing is compiled on the server.
</p>

<h2>1. Install it</h2>
<p>
  You need Linux on x86-64 with systemd and Docker Engine 28 or newer, with Compose and Buildx. Ask your coding agent
  on the server:
</p>
<Code lang="prompt" code={prompt} />
<p>Or run the installer yourself, after reading it:</p>
<Code lang="shell" code={install} />
<p>It runs as your own account and, in order:</p>
<ol>
  <li>checks the server: Docker, memory, disk and port 9443;</li>
  <li>clones easi.run into <code>~/easi.run</code> at the release it names;</li>
  <li>
    puts the firewall rule that keeps pages off the server’s own services in place, and makes Docker put it back
    before every start, so Docker and its containers start only once it is there;
  </li>
  <li>
    pulls the images easi.run’s CI built from that release, each checked against the digest the installer names
    (about 350 MB), and adds Google’s current Chrome stable to the browser image from Google’s signed repository
    (about 150 MB): Google’s terms let no one else ship Chrome. On 2 cores with a fast connection, this takes one to two
    minutes;
  </li>
  <li>starts easi.run, and checks it over HTTPS against the admin’s own certificate;</li>
  <li>
    offers easi.run to Claude Code over MCP (see <a href="#/agents">Agents</a>);
  </li>
  <li>
    prints a summary: whether the admin’s password is set, whether MCP is ready, and the lines for
    another computer, the SSH tunnel’s among them.
  </li>
</ol>
<p>
  <code>bash install.sh --dry-run</code> runs the checks and lists the images it would pull, changing nothing;
  <code>bash install.sh --update</code>, with a newer installer, moves an installation to its release.
</p>

<h2>2. Open it</h2>
<p>
  The admin is at <code>https://localhost:9443</code>, on the server’s <code>127.0.0.1</code> only. From your
  computer, use the command you already use to log in to the server, adding <code>-N -L 9443:127.0.0.1:9443</code>,
  and keep it running; then open <code>https://localhost:9443</code>. For example:
</p>
<Code lang="shell" code={tunnel} />
<p>
  The installer’s summary prints this line, filled in with your login when it can tell it. VS Code and Cursor over
  Remote-SSH forward the port for you: open the link from their terminal, or add port 9443 in their Ports view; either
  way, the local port must be 9443 too.
</p>
<p>
  The certificate is self-signed: the installer keeps a copy in <code>~/.config/easi-run/tls-cert.pem</code>, and
  the admin logs its SHA-256 fingerprint at start, so you can check it before you accept it.
</p>

<h2>3. Create your password and start a session</h2>
<ol>
  <li>
    Open <code>https://localhost:9443</code> (from your computer, through the tunnel) right after the install: until a
    password is set, the admin’s first screen asks for one. Choose it, 12 characters or more; you are logged in at
    once.
  </li>
  <li>
    Next, add a second step at sign-in, or skip it for now. It is optional, and we suggest it: with it on, your
    password alone opens nothing. An <strong>authenticator app</strong> works in every browser: scan the QR code, type
    the 6-digit code it shows, and each code signs in once. A <strong>passkey</strong>, where your browser offers one,
    signs you in on its own with your fingerprint, face or device PIN; Chrome offers none on a site whose certificate
    it does not trust, such as the server’s own over the tunnel, and the page then says so. Save the ten
    <strong>recovery codes</strong> it shows once: if you lose your phone or passkey, sign in with your password and one
    of them; each works once. Change it
    later under <strong>Security</strong>.
  </li>
  <li>Type a URL and press <strong>Start session</strong>, or ask your agent to open one.</li>
  <li>Watch the page, or press <strong>Take control</strong> to use it yourself, then <strong>Give back</strong>.</li>
</ol>

<h2>Without the installer</h2>
<p>
  First, the firewall rule, which keeps pages off the services on every address the host holds (see
  <a href="#/security">Security</a>). Keep it across reboots with your host’s firewall configuration.
</p>
<Code lang="shell" code={firewall} />
<p>Then start easi.run:</p>
<Code lang="shell" code={run} />
<p>
  Then open <code>https://localhost:9443</code> and choose your password, 12 characters or more. <code>DOCKER_GID</code> gives the controller
  the group of the Docker socket; without it, no session can start.
</p>

<Note>
  Each browser session gets 2 CPUs and 2048 MiB by default. The installer runs as many at once as the server’s memory
  holds (one on 4 GB, two on 8 GB, four on 16 GB, up to 16); by hand, one runs at once. Change that with <code>BROWSER_CPUS</code>, <code>BROWSER_MEMORY_MIB</code> and <code>ADMIN_MAX_SESSIONS</code>; see
  <a href="#/configuration">Configuration</a>.
</Note>
