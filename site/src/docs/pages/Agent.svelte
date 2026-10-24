<script lang="ts">
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  const call = `BASE=https://localhost:9443
# Log in once: keep the cookie and the CSRF token it returns.
CSRF=$(curl -sk -c jar -H "Origin: $BASE" -H 'Content-Type: application/json' \\
  -d '{"password":"your password"}' $BASE/api/login | jq -r .csrf)

# One tool call on session 0123456789ab.
curl -sk -b jar -H "Origin: $BASE" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \\
  -d '{"tool":"get_desktop_state","arguments":{}}' \\
  $BASE/api/sessions/0123456789ab/agent`;
</script>

<h1 tabindex="-1">The agent</h1>
<p class="lead">
  Each session runs the <a href="https://github.com/trycua/cua">Cua Driver</a> as its agent: it reads the page through
  the accessibility tree and acts through the compositor’s virtual input.
</p>

<h2>How you reach it</h2>
<p>
  The admin speaks MCP to the driver over its standard streams, inside the session, and exposes it as one
  authenticated HTTP call: <code>POST /api/sessions/{'{id}'}/agent</code> with <code>{'{"tool", "arguments"}'}</code>.
  The response is the tool’s MCP result.
</p>
<Code lang="shell" code={call} />
<p>An agent of your own reaches its sessions with a token instead of your login: see <a href="#/agents">Agents</a>.</p>
<ul>
  <li>Every call needs the login cookie, the exact <code>Origin</code>, and the <code>X-CSRF-Token</code> from the login.</li>
  <li>One call runs at a time per session, for up to 120 seconds; up to 4 more wait, each up to 30 seconds, and any more are refused with <code>503</code>.</li>
  <li>While a person holds control, calls are refused with <code>409</code>.</li>
  <li>After the first takeover in a session, results come back without screenshots, and what the person typed is redacted from the text.</li>
</ul>

<h2>Tools</h2>
<p>The tools are the Cua Driver’s own. These work in every session:</p>
<p>
  <code>list_windows</code>, <code>get_window_state</code>, <code>get_desktop_state</code>, <code>get_app</code>,
  <code>click</code>, <code>type_text</code>, <code>press_key</code>, <code>hotkey</code>, <code>scroll</code>.
</p>
<p>
  These need the session’s <strong>DevTools</strong> option: <code>browser_navigate</code>, <code>browser_click</code>,
  <code>browser_type</code>, <code>browser_prepare</code>.
</p>

<Note>
  DevTools makes the agent run as Chrome’s uid, in Chrome’s network namespace, so it can reach the debugging endpoint:
  turn it on only for sessions whose agent needs the browser tools.
</Note>

<h2>When a person takes over</h2>
<ol>
  <li>The admin asks the agent to stop and waits up to 5 seconds for proof that it did.</li>
  <li>Without that proof the agent’s container is killed, and it starts again when control comes back.</li>
  <li>From then on, only the person’s input reaches the browser until they give back.</li>
</ol>
