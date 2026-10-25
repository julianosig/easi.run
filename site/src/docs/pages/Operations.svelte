<script lang="ts">
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  const update = 'curl -fsSLO https://easi.run/install.sh && bash install.sh --update';
  const byHand = `cd ~/easi.run && git pull
export DOCKER_GID=$(stat -c %g /var/run/docker.sock)
docker compose up -d --build
docker compose up -d --no-deps --force-recreate controller admin`;
  const remove = 'bash ~/easi.run/install.sh --uninstall';
  const reset = 'docker exec easi-run-admin-1 admin reset-second-step';
  const domain = `# compose.override.yaml, next to compose.yaml
services:
  admin:
    environment:
      ADMIN_ORIGIN: https://browser.example.com
      ADMIN_HOSTNAMES: browser.example.com,localhost,127.0.0.1`;
  const logs = `cd ~/easi.run
docker compose logs -f admin controller
docker compose logs admin | grep -i fingerprint`;
</script>

<h1 tabindex="-1">Operations</h1>
<p class="lead">Updating, removing, reaching the admin from elsewhere, and what restarts do.</p>

<h2>Update</h2>
<p>With the installer: the latest one moves your installation to the latest release.</p>
<Code lang="shell" code={update} />
<p>
  Every run of the installer, with <code>--update</code> or without, also builds Google’s newest Chrome stable from
  Google’s signed repository when there is one, and then starts the controller and the admin afresh, which ends the
  sessions running at the time.
</p>
<p>
  Installed by hand: in the checkout, pull, rebuild, then start the controller and the admin afresh, so new sessions
  use the new images. Compose reads <code>compose.override.yaml</code> on its own; with an egress file, keep
  <code>COMPOSE_FILE</code> in <code>.env</code> (see <a href="#/configuration">Configuration</a>), or the admin
  starts again without its egresses.
</p>
<Code lang="shell" code={byHand} />
<Note tone="warn">
  Restarting the admin stops every running session and ends every login: it removes whatever an earlier run left
  behind before it accepts new sessions.
</Note>

<h2>Remove</h2>
<Code lang="shell" code={remove} />
<p>
  After you confirm (<code>--yes</code> confirms ahead, for an agent), the installer removes what it put on the server:
  easi.run’s containers, with every session, its volumes, networks and images, the <code>docker.service</code> drop-in
  and the firewall rule (unless another easi.run installation on the server still needs them),
  <code>~/.config/easi-run</code>, and Claude Code’s <code>easi</code> server when it is this
  installation’s. It keeps the checkout, <code>~/easi.run</code>, with any <code>.env</code>, override or egress file
  in it, and Docker’s build cache, which <code>docker builder prune</code> removes for every project on the server.
</p>

<h2>Reaching the admin</h2>
<ul>
  <li>By default the Compose file publishes it on the host’s <code>127.0.0.1:9443</code> only. From another machine, use an SSH tunnel (<code>ssh -L 9443:127.0.0.1:9443 you@server</code>).</li>
  <li>Behind your own domain or reverse proxy, set <code>ADMIN_ORIGIN</code> to the exact address people open and add the name to <code>ADMIN_HOSTNAMES</code>, in an override file: <code>compose.yaml</code> sets both itself. Requests from any other origin are refused.</li>
  <li>To use your own certificate, put <code>tls-cert.pem</code> and <code>tls-key.pem</code> in the admin’s state volume.</li>
</ul>
<Code lang="yaml" code={domain} />
<p>
  After you change <code>ADMIN_HOSTNAMES</code>, the next start makes the admin's own certificate again for the new
  names. A certificate you put in the state volume yourself is never replaced.
</p>

<h2>A lost second step</h2>
<p>
  Lost the phone with the authenticator app, or the passkey, and the recovery codes too? On the server, this turns
  the second step off; the password alone signs in again at once, without a restart, and you can turn it on again
  under <strong>Security</strong>:
</p>
<Code lang="shell" code={reset} />

<h2>Logs</h2>
<p>
  <code>docker compose</code> finds the installation from its checkout, <code>~/easi.run</code>; from anywhere else,
  <code>docker logs easi-run-admin-1</code> and <code>docker logs easi-run-controller-1</code> show the same.
</p>
<Code lang="shell" code={logs} />
<p>
  The admin logs its certificate’s fingerprint on every start. Until a password is set, the setup link that sets it is
  never logged: <code>docker compose exec admin admin setup</code> prints it with 5 minutes left at least, and
  says how long it lasts; the admin makes a new one 5 minutes before a link expires, and the older link works until
  then.
</p>
