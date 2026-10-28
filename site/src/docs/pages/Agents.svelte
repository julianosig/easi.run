<script lang="ts">
  import Rich from '../Rich.svelte';
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  const local = 'claude mcp add --scope user easi -- docker exec -i easi-run-admin-1 admin mcp';
  const remote = 'claude mcp add --scope user easi -- ssh -T you@your-server docker exec -i easi-run-admin-1 admin mcp';
  const codex = 'codex mcp add easi -- docker exec -i easi-run-admin-1 admin mcp';
  const claude = `claude mcp add --transport http easi https://localhost:9443/mcp \\
  --header "Authorization: Bearer easi_…"`;
  const certificate = `# The admin's self-signed certificate, for clients that check it (Node-based ones do).
# On the server, the installer keeps a copy (installed by hand? in ~/easi.run, make it with
# mkdir -p ~/.config/easi-run && docker compose cp admin:/var/lib/admin/tls-cert.pem ~/.config/easi-run/tls-cert.pem):
export NODE_EXTRA_CA_CERTS=$HOME/.config/easi-run/tls-cert.pem
# On another computer, copy it from the server first:
scp you@your-server:.config/easi-run/tls-cert.pem easi-run.pem
export NODE_EXTRA_CA_CERTS=$PWD/easi-run.pem`;
  const rest = `BASE=https://localhost:9443
AUTH="Authorization: Bearer easi_…"

# Start a session: its id, and a watch_url for you.
curl -sk -H "$AUTH" -H 'Content-Type: application/json' \\
  -d '{"url":"https://example.com"}' $BASE/api/agent/sessions

# One tool call: the body is the tool's own arguments.
curl -sk -H "$AUTH" -H 'Content-Type: application/json' -d '{}' \\
  $BASE/api/agent/sessions/0123456789ab/tools/get_desktop_state

# Its state (controlled: a person holds it), then stop it.
curl -sk -H "$AUTH" $BASE/api/agent/sessions/0123456789ab
curl -sk -H "$AUTH" -X DELETE $BASE/api/agent/sessions/0123456789ab`;
  const routes: [string, string, string][] = [
    ['POST', '/api/agent/sessions', 'agents.route.post-api-agent-sessions'],
    ['GET', '/api/agent/saved-sessions', 'agents.route.get-api-agent-saved-sessions'],
    ['GET', '/api/agent/sessions', 'agents.route.get-api-agent-sessions'],
    ['GET', '/api/agent/sessions/{id}', 'agents.route.get-api-agent-sessions-id'],
    ['DELETE', '/api/agent/sessions/{id}', 'agents.route.delete-api-agent-sessions-id'],
    ['GET', '/api/agent/tools', 'agents.route.get-api-agent-tools'],
    ['POST', '/api/agent/sessions/{id}/tools/{tool}', 'agents.route.post-api-agent-sessions-id-tools-tool'],
  ];
</script>

<h1 tabindex="-1"><Rich message="agents.title" /></h1>
<p class="lead"><Rich message="agents.intro.1" /></p>

<h2><Rich message="agents.mcp-on-the-server.title" /></h2>
<p><Rich message="agents.mcp-on-the-server.1" /></p>
<Code lang="shell" code={local} />
<Code lang="shell" code={remote} />
<Code lang="shell" code={codex} />
<ul>
  <li><Rich message="agents.mcp-on-the-server.2" /></li>
  <li><Rich message="agents.mcp-on-the-server.3" /></li>
  <li><Rich message="agents.mcp-on-the-server.4" /></li>
</ul>

<h2><Rich message="agents.tokens.title" /></h2>
<ul>
  <li><Rich message="agents.tokens.1" /></li>
  <li><Rich message="agents.tokens.2" /></li>
  <li><Rich message="agents.tokens.3" /></li>
  <li><Rich message="agents.tokens.4" /></li>
  <li><Rich message="agents.tokens.5" /></li>
  <li><Rich message="agents.tokens.6" /></li>
</ul>

<h2><Rich message="agents.your-logins-for-an.title" /></h2>
<ul>
  <li><Rich message="agents.your-logins-for-an.1" /></li>
  <li><Rich message="agents.your-logins-for-an.2" /></li>
</ul>

<h2><Rich message="agents.mcp-over-https.title" /></h2>
<p><Rich message="agents.mcp-over-https.1" /></p>
<Code lang="shell" code={claude} />
<Code lang="shell" code={certificate} />
<p><Rich message="agents.mcp-over-https.2" /></p>
<ul>
  <li><Rich message="agents.mcp-over-https.3" /></li>
  <li><Rich message="agents.mcp-over-https.4" /></li>
  <li><Rich message="agents.mcp-over-https.5" /></li>
  <li><Rich message="agents.mcp-over-https.6" /></li>
</ul>

<h2><Rich message="agents.http-api.title" /></h2>
<table>
  <thead><tr><th><Rich message="agents.http-api.1" /></th><th><Rich message="agents.http-api.2" /></th></tr></thead>
  <tbody>
    {#each routes as [method, path, what] (method + path)}
      <tr><td><code>{method} {path}</code></td><td><Rich message={what} /></td></tr>
    {/each}
  </tbody>
</table>
<Code lang="shell" code={rest} />
<p><Rich message="agents.http-api.4" /></p>

<h2><Rich message="agents.when-you-take-over.title" /></h2>
<ul>
  <li><Rich message="agents.when-you-take-over.1" /></li>
  <li><Rich message="agents.when-you-take-over.2" /></li>
</ul>

<Note><Rich message="agents.when-you-take-over.3" /></Note>
