<script lang="ts">
  const routes: [string, string, string, string][] = [
    ['GET', '/api/status', 'Whether a password is set; the demo’s sites and length in demo mode.', 'none'],
    ['POST', '/api/enroll', '{token, password}: sets the password with the enrollment token.', 'Origin'],
    ['POST', '/api/login', '{password}; in the demo {demo?, wait?}: the demo to start at, and wait (a start the page makes on its own) to wait for this client’s visit open elsewhere rather than close it. Sets the cookie, returns {csrf} (and the demo visit).', 'Origin'],
    ['POST', '/api/demo/switch', '{demo, viewer}: in the public demo, moves the visitor’s turn to another demo; that viewer must hold control.', 'login + CSRF'],
    ['POST', '/api/logout', 'Ends the login and its streams.', 'login + CSRF'],
    ['GET', '/api/me', 'The login’s CSRF token (and the demo visit).', 'login'],
    ['GET', '/api/egresses', 'The egresses’ names, time zones and languages.', 'login'],
    ['GET', '/api/sessions', 'The running sessions, each with its features.', 'login'],
    ['POST', '/api/sessions', '{url, devtools?, egress?, features?}: starts a session (see Custom clients).', 'login + CSRF'],
    ['DELETE', '/api/sessions/{id}', 'Stops a session.', 'login + CSRF'],
    ['POST', '/api/sessions/{id}/control', '{viewer, if_free?}: takes control for that viewer.', 'login + CSRF'],
    ['POST', '/api/sessions/{id}/release', '{viewer}: gives control back to the agent.', 'login + CSRF'],
    ['PUT', '/api/sessions/{id}/settings', '{codec, bitrate_kbps, max_fps}: the stream’s encoding.', 'login + CSRF'],
    ['PUT', '/api/sessions/{id}/network', '{latency_ms, loss_percent, bandwidth_kbps}: a simulated network, for testing.', 'login + CSRF'],
    ['POST', '/api/sessions/{id}/agent', '{tool, arguments}: one agent tool call.', 'login + CSRF'],
    ['GET', '/api/sessions/{id}/stats', 'The session, its containers’ usage and its viewers.', 'login'],
    ['GET', '/api/sessions/{id}/files/{ticket}', 'A download, by its single-use ticket.', 'login'],
    ['GET', '/ws/sessions/{id}?viewer=…', 'The viewer stream (WebSocket, binary protocol).', 'Origin + login'],
    ['GET', '/api/tokens', 'The agent tokens: id, name, created, last used; never the token.', 'login'],
    ['POST', '/api/tokens', '{name}: creates an agent token and returns it, this once.', 'login + CSRF'],
    ['DELETE', '/api/tokens/{id}', 'Revokes a token and stops every session it started.', 'login + CSRF'],
  ];
</script>

<h1 tabindex="-1">HTTP API</h1>
<p class="lead">The admin’s API, the same one its web client uses. JSON in and out.</p>

<h2>Authentication</h2>
<ul>
  <li>Log in with <code>POST /api/login</code>: it sets the <code>__Host-display</code> cookie and returns a CSRF token.</li>
  <li>Every request that changes something needs the exact <code>Origin</code> (<code>ADMIN_ORIGIN</code>) and the <code>X-CSRF-Token</code> header.</li>
  <li>A login ends after 30 minutes idle or 12 hours in all.</li>
  <li>After 5 wrong passwords in a row, the next attempt waits 2 seconds, and each further wrong one doubles the wait, up to 256 seconds (<code>429</code> until then).</li>
</ul>

<p>Agents holding a token use their own routes under <code>/api/agent/</code> and the MCP endpoint at <code>/mcp</code> instead: see <a href="#/agents">Agents</a>.</p>

<h2>Routes</h2>
<table>
  <thead><tr><th>Route</th><th>What it does</th><th>Needs</th></tr></thead>
  <tbody>
    {#each routes as [method, path, what, needs] (method + path)}
      <tr><td><code>{method} {path}</code></td><td>{what}</td><td class="whitespace-nowrap text-muted">{needs}</td></tr>
    {/each}
  </tbody>
</table>

<h2>Encoding settings</h2>
<table>
  <thead><tr><th>Field</th><th>Values</th></tr></thead>
  <tbody>
    <tr><td><code>codec</code></td><td><code>auto</code>, <code>av1</code>, <code>h264</code> (only in builds with H.264), <code>tiles_only</code></td></tr>
    <tr><td><code>bitrate_kbps</code></td><td>100–50000, default 2500</td></tr>
    <tr><td><code>max_fps</code></td><td>1–120, default 60</td></tr>
  </tbody>
</table>
