<script lang="ts">
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  const update = `git pull
DOCKER_GID=$(stat -c %g /var/run/docker.sock) docker compose up -d --build
docker compose restart controller`;
  const domain = `# compose.override.yaml, next to compose.yaml
services:
  admin:
    environment:
      ADMIN_ORIGIN: https://browser.example.com
      ADMIN_HOSTNAMES: browser.example.com,localhost,127.0.0.1`;
  const logs = `docker compose logs -f admin controller
docker compose logs admin | grep -i fingerprint`;
</script>

<h1 tabindex="-1">Operations</h1>
<p class="lead">Updating, reaching the admin from elsewhere, and what restarts do.</p>

<h2>Update</h2>
<Code lang="shell" code={update} />
<Note tone="warn">
  Restarting the admin stops every running session and ends every login: it removes whatever an earlier run left
  behind before it accepts new sessions.
</Note>

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

<h2>Logs</h2>
<Code lang="shell" code={logs} />
<p>
  The admin logs its certificate’s fingerprint on every start and, until a password is set, a new enrollment token on
  every start, each valid for 15 minutes: restart the admin (<code>docker compose restart admin</code>) for a new one.
</p>
