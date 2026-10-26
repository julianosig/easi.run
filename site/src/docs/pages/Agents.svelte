<script lang="ts">
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
    ['POST', '/api/agent/sessions', '{url, egress?, resume?}: starts a session, through an egress by its name (refused with 424 {reason, detail} when its check fails), or with resume, resumes the saved session of that id this token may use; returns it with its watch_url and saved_session (the one it resumed, or null).'],
    ['GET', '/api/agent/saved-sessions', 'The saved sessions this token may resume: id, name, running; and whether they are unlocked.'],
    ['GET', '/api/agent/sessions', 'The sessions this token started.'],
    ['GET', '/api/agent/sessions/{id}', 'One of them: its URL, controlled (a person holds it), failed, watch_url.'],
    ['DELETE', '/api/agent/sessions/{id}', 'Stops it.'],
    ['GET', '/api/agent/tools', 'The desktop tools, as the driver defines them.'],
    ['POST', '/api/agent/sessions/{id}/tools/{tool}', 'The body is the tool’s arguments; the answer is its MCP result.'],
  ];
</script>

<h1 tabindex="-1">Agents</h1>
<p class="lead">
  Give an agent a token, and it starts its own sessions and drives them, over MCP or plain HTTP. You watch any of them
  from the admin page and take over when you want.
</p>

<h2>MCP on the server</h2>
<p>
  The installer offers easi.run to Claude Code with this line; once its summary says MCP is ready, easi.run is in
  Claude Code from its next session. It is the line for any MCP client that runs a command (stdio), here or from
  another computer over SSH, with the login you already use for the server in place of <code>you@your-server</code>:
</p>
<Code lang="shell" code={local} />
<Code lang="shell" code={remote} />
<Code lang="shell" code={codex} />
<ul>
  <li>
    <code>admin mcp</code> relays the client’s stdio to the running admin over a socket in its own state directory
    (mode 0600, answered only for the admin’s own user). No token, port or certificate is involved: whoever can run
    <code>docker exec</code> on the server can use it, and SSH carries it from another computer.
  </li>
  <li>
    Its clients have an agent token’s rights, together: they start sessions and drive the ones they started, and the
    admin page shows those sessions like any token’s.
  </li>
  <li>The same tools as below. <code>ADMIN_LOCAL_MCP=0</code> turns the socket off; the public demo has none.</li>
</ul>

<h2>Tokens</h2>
<ul>
  <li>Create one under <strong>Agent tokens</strong> on the admin page (or <code>POST /api/tokens {'{"name"}'}</code>). It is shown once: copy it then.</li>
  <li>A token starts sessions and uses their desktop tools. It reaches only the sessions it started: never yours, another token’s, the viewer stream, downloads, takeovers, settings or tokens.</li>
  <li>Requests carry it as <code>Authorization: Bearer easi_…</code>, with no cookie or CSRF token. A request from a page on another site (another <code>Origin</code>) is refused.</li>
  <li>Revoking a token refuses it at once and stops every session it started, one still starting included.</li>
  <li>A token’s sessions count toward <code>ADMIN_MAX_SESSIONS</code>, with yours: close them from the session list, or revoke the token, to free their places.</li>
  <li>The admin keeps only each token’s SHA-256, in <code>tokens.json</code> in its state volume. Up to 32 tokens; none in the public demo.</li>
</ul>

<h2>Your logins for an agent</h2>
<ul>
  <li>A token, or the local MCP clients, may resume the <a href="#/sessions">persistent sessions</a> you allow it. It then acts on those sites as you, logged in, and stopping the session saves it again.</li>
  <li><code>list_saved_sessions</code> (or <code>GET /api/agent/saved-sessions</code>) lists them as saved sessions; pass one’s <code>id</code> as <code>resume</code> to <code>start_session</code>. Any other answers “no such saved session”.</li>
</ul>

<h2>MCP over HTTPS</h2>
<p>
  For clients that connect over HTTP with a token, the endpoint is <code>/mcp</code>: Streamable HTTP, JSON answers, no session state. It serves MCP <code>2026-07-28</code>
  and, for clients that open with <code>initialize</code>, <code>2025-11-25</code> and <code>2025-06-18</code>. With
  Claude Code (tested with 2.1.303):
</p>
<Code lang="shell" code={claude} />
<Code lang="shell" code={certificate} />
<p>The tools:</p>
<ul>
  <li><code>start_session {'{url, egress?, resume?}'}</code>: returns the session’s <code>id</code> and its <code>watch_url</code>; <code>resume</code> resumes a saved session.</li>
  <li><code>list_sessions</code>, <code>session_status {'{session}'}</code>, <code>stop_session {'{session}'}</code>.</li>
  <li><code>list_saved_sessions</code>: the saved sessions it may resume.</li>
  <li>The driver’s desktop tools (<a href="#/agent">The agent</a> lists them), each taking <code>{'{session, arguments}'}</code>, its own arguments under <code>arguments</code>.</li>
</ul>

<h2>HTTP API</h2>
<table>
  <thead><tr><th>Route</th><th>What it does</th></tr></thead>
  <tbody>
    {#each routes as [method, path, what] (method + path)}
      <tr><td><code>{method} {path}</code></td><td>{what}</td></tr>
    {/each}
  </tbody>
</table>
<Code lang="shell" code={rest} />
<p>A session’s agent runs one call at a time; <a href="#/agent">The agent</a> says how many may wait, and for how long.</p>

<h2>When you take over</h2>
<ul>
  <li>Open the session from its <code>watch_url</code> (it logs you in first) or from the session list. If nobody holds control and your agent is not using the session (no call running, none ended in the last 30 seconds), you usually hold control as it opens; otherwise you watch and press <strong>Take control</strong>. <a href="#/agent">The agent</a> lists every case.</li>
  <li>While you hold control, its desktop tools are refused (<code>409</code> over HTTP, <code>isError</code> over MCP; <code>controlled</code> says so), and they work again when you give it back. <a href="#/agent">The agent</a> says what a takeover does to its calls and results.</li>
</ul>

<Note>
  The tools are the ones a session without DevTools runs, pinned from the driver’s own definitions. Sessions a token
  starts never have DevTools.
</Note>
