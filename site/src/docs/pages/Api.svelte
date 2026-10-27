<script lang="ts">
  const routes: [string, string, string, string][] = [
    ['GET', '/api/status', 'Whether a password is set, and whether a passkey is added (the login offers it); the demo’s sites and length in demo mode.', 'none'],
    ['POST', '/api/enroll', '{password}: sets the first password while none is set (403 once one is, and in the demo), and logs in as /api/login does: sets the cookie, returns {csrf}.', 'Origin'],
    ['POST', '/api/login', '{password}; in the demo {demo?, wait?}: the demo to start at, and wait (a start the page makes on its own) to wait for this client’s visit open elsewhere rather than close it. Sets the cookie, returns {csrf} (and the demo visit). With the second step on, sets no cookie and returns {second_step: {step, app, passkey}}: a pending step for /api/login/code, and what may answer it.', 'Origin'],
    ['POST', '/api/login/code', '{step, code}: answers the pending step with a code from the authenticator app or a recovery code. Sets the cookie, returns {csrf}.', 'Origin'],
    ['POST', '/api/login/passkey', 'Begins a passkey sign-in: returns {ceremony, options}, the options for navigator.credentials.get.', 'Origin'],
    ['POST', '/api/login/passkey/finish', '{ceremony, credential}: the credential as JSON, binary fields in base64url. Sets the cookie, returns {csrf}.', 'Origin'],
    ['POST', '/api/demo/switch', '{demo, viewer}: in the public demo, moves the visitor’s turn to another demo; that viewer must hold control.', 'login + CSRF'],
    ['POST', '/api/logout', 'Ends the login and its streams.', 'login + CSRF'],
    ['GET', '/api/me', 'The login’s CSRF token (and the demo visit).', 'login'],
    ['GET', '/api/egresses', 'The egresses: {max, egresses: [{id, name, scheme, host, port, has_user, has_password, timezone, languages, country, country_name, in_use}]}; never a credential.', 'login'],
    ['GET', '/api/egresses/options', 'The time zones and languages offered first: {timezones, languages: [{tag, name}]}.', 'login'],
    ['POST', '/api/egresses', '{name, proxy, timezone?, languages?}: checks the proxy and saves it only if the check passed, with the country it found and, unless given, that country’s time zone and languages; 422 {reason, detail} otherwise.', 'login + CSRF'],
    ['PATCH', '/api/egresses/{id}', '{name?, proxy?, timezone?, languages?}: a new proxy is checked first and replaces the old one, credentials included, only if it passed.', 'login + CSRF'],
    ['DELETE', '/api/egresses/{id}', 'Deletes it, unless a session uses it (409, naming the sessions).', 'login + CSRF'],
    ['POST', '/api/egresses/check', '{proxy}: checks a proxy without saving it: {ok: true, latency_ms, exit: {ip, country, country_name}, suggested: {timezone, languages}} or {ok: false, reason, detail}.', 'login + CSRF'],
    ['POST', '/api/egresses/{id}/check', 'Checks a saved egress, the same way.', 'login + CSRF'],
    ['GET', '/api/sessions', 'The running sessions, each with its features.', 'login'],
    ['POST', '/api/sessions', '{url, devtools?, egress?, features?, profile?, width?, height?}: starts a session, through an egress by its id (refused with 424 {reason, detail} when its check fails); with profile, resumes the persistent session of that id (see Custom clients); with width and height (even, 320x240 to 1920x1080, and not a size SVT-AV1 crashes on: one side 2–8 pixels past a multiple of 64, the other 10–16; refused with 400 otherwise), its screen is that size instead of ADMIN_OUTPUT.', 'login + CSRF'],
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
    ['GET', '/api/profiles', 'The persistent sessions, each a profile here: {unlocked, max, profiles: [{id, name, allowed (token ids, and local), created, used, size, session (its run now, or null), unsaved}]}.', 'login'],
    ['POST', '/api/profiles', '{name}: makes a persistent session that has not run yet; refused (423) while they are locked.', 'login + CSRF'],
    ['PUT', '/api/profiles/{id}', '{name?, allowed?}: renames it, or sets the tokens (and local) that may resume it.', 'login + CSRF'],
    ['DELETE', '/api/profiles/{id}', 'Deletes it and what it kept, unless it is running.', 'login + CSRF'],
    ['POST', '/api/profiles/unlock', '{password}: unlocks the persistent sessions after a sign-in that checked no password (a passkey’s).', 'login + CSRF'],
    ['GET', '/api/security', 'The second step: {app, recovery_codes (how many are left), passkeys: [{id, created}], passkeys_possible}.', 'login'],
    ['POST', '/api/security/app', 'Draws a new secret for an authenticator app: returns {secret, uri}; nothing changes until it is confirmed.', 'login + CSRF'],
    ['POST', '/api/security/app/confirm', '{code, password?}: turns the app on with a code from it; replacing one that is on needs the password. Returns {recovery_codes}, the codes when the second step was off until now, else null.', 'login + CSRF'],
    ['POST', '/api/security/app/remove', '{password}: turns the app off.', 'login + CSRF'],
    ['POST', '/api/security/passkeys', '{password}: begins adding a passkey: returns {options}, for navigator.credentials.create.', 'login + CSRF'],
    ['POST', '/api/security/passkeys/finish', '{credential}: adds it. Returns {recovery_codes}, as the app’s confirmation does.', 'login + CSRF'],
    ['POST', '/api/security/passkeys/{id}/remove', '{password}: removes that passkey.', 'login + CSRF'],
    ['POST', '/api/security/recovery', '{password}: new recovery codes in place of those left; returns {recovery_codes}.', 'login + CSRF'],
  ];
</script>

<h1 tabindex="-1">HTTP API</h1>
<p class="lead">The admin’s API, the same one its web client uses. JSON in and out.</p>

<h2>Authentication</h2>
<ul>
  <li>Log in with <code>POST /api/login</code>: it sets the <code>__Host-display</code> cookie and returns a CSRF token.</li>
  <li>Every request that changes something needs the exact <code>Origin</code> (<code>ADMIN_ORIGIN</code>), and once logged in the <code>X-CSRF-Token</code> header too (setting the password and logging in come before a login).</li>
  <li>A login ends after 30 minutes idle or 12 hours in all.</li>
  <li>After 5 wrong passwords in a row, the next attempt waits 2 seconds, and each further wrong one doubles the wait, up to 256 seconds (<code>429</code> until then). Wrong codes and refused passkeys count the same, and only a completed login resets the count.</li>
  <li>With the second step on, the password returns a pending step instead of a login: answer it with <code>POST /api/login/code</code> within 2 minutes. A step that is used, expired, or refused five codes answers <code>401</code> with <code>reason: "again"</code>: log in with the password again.</li>
  <li>A wrong current password on a <code>/api/security/</code> route answers <code>403</code> with <code>reason: "password"</code>; the login stays. Every change there ends every other login.</li>
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
