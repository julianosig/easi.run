<script lang="ts">
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  import { TOOLS } from '../tools.ts';
  const claude = `claude mcp add --transport http easi https://localhost:9443/mcp \\
  --header "Authorization: Bearer easi_…"`;
  const certificate = `# The admin's self-signed certificate, for clients that check it (Node-based ones do).
docker compose cp admin:/var/lib/admin/tls-cert.pem easi-run.pem
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
    ['POST', '/api/agent/sessions', '{url, egress?}: starts a session; returns it with its watch_url.'],
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

<h2>Tokens</h2>
<ul>
  <li>Create one under <strong>Agent tokens</strong> on the admin page (or <code>POST /api/tokens {'{"name"}'}</code>). It is shown once: copy it then.</li>
  <li>A token starts sessions and uses their desktop tools. It reaches only the sessions it started: never yours, another token’s, the viewer stream, downloads, takeovers, settings or tokens.</li>
  <li>Requests carry it as <code>Authorization: Bearer easi_…</code>, with no cookie or CSRF token. A request from a page on another site (another <code>Origin</code>) is refused.</li>
  <li>Revoking a token refuses it at once and stops every session it started, one still starting included.</li>
  <li>A token’s sessions count toward <code>ADMIN_MAX_SESSIONS</code>, with yours: close them from the session list, or revoke the token, to free their places.</li>
  <li>The admin keeps only each token’s SHA-256, in <code>tokens.json</code> in its state volume. Up to 32 tokens; none in the public demo.</li>
</ul>

<h2>MCP</h2>
<p>
  The endpoint is <code>/mcp</code>: Streamable HTTP, JSON answers, no session state. It serves MCP <code>2026-07-28</code>
  and, for clients that open with <code>initialize</code>, <code>2025-11-25</code> and <code>2025-06-18</code>. With
  Claude Code (tested with 2.1.303):
</p>
<Code lang="shell" code={claude} />
<Code lang="shell" code={certificate} />
<p>The tools:</p>
<ul>
  <li><code>start_session {'{url, egress?}'}</code>: returns the session’s <code>id</code> and its <code>watch_url</code>.</li>
  <li><code>list_sessions</code>, <code>session_status {'{session}'}</code>, <code>stop_session {'{session}'}</code>.</li>
  <li>
    The driver’s desktop tools, each taking <code>{'{session, arguments}'}</code>, its own arguments under
    <code>arguments</code>: {#each TOOLS as t, i (t)}<code>{t}</code>{i < TOOLS.length - 1 ? ', ' : '.'}{/each}
  </li>
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
<p>
  A session’s agent runs one call at a time; up to 4 more may wait, each for up to 30 seconds. Beyond that a call is
  refused without running: <code>503</code> with <code>Retry-After</code>, or <code>isError</code> over MCP.
</p>

<h2>When you take over</h2>
<ul>
  <li>Open the session from its <code>watch_url</code> (it logs you in first) or from the session list.</li>
  <li>While you hold control, its desktop tools are refused: <code>409</code> over HTTP, a result with <code>isError</code> over MCP. <code>controlled</code> says so.</li>
  <li>When you give it back, they work again. From your first takeover on, results carry no screenshots, and what you typed or pasted is redacted (see <a href="#/security">Security</a>).</li>
</ul>

<Note>
  The tools are the ones a session without DevTools runs, pinned from the driver’s own definitions. Sessions a token
  starts never have DevTools.
</Note>
