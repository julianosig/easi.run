<script lang="ts">
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  import { TOOLS } from '../tools.ts';
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
  <li>One call runs at a time per session; up to 4 more wait, each up to 30 seconds, and any more are refused without running: <code>503</code> with <code>Retry-After</code>, or <code>isError</code> over MCP.</li>
  <li>Once a call is sent to the agent, it fails if the agent has not answered within 150 seconds. A <code>run_steps</code> batch gets the longer of 120 seconds and the time its steps may take, plus 30 seconds, and at most 600 seconds.</li>
  <li>An answer comes back when it is at most 16 MiB as the agent sends it and at most 48 MiB once parsed, as the admin counts it (a long array of small values reaches 48 MiB before 16 MiB), and when reading and parsing it, its escaped strings decoded, take at most 88 MiB together. A larger one fails the call, and the agent starts afresh for the next.</li>
  <li>After the first takeover in a session, results come back redacted and without screenshots (see <a href="#/security">Security</a>).</li>
</ul>

<h2>Tools</h2>
<p>The tools are the Cua Driver’s own. These work in every session:</p>
<p>{#each TOOLS as t, i (t)}<code>{t}</code>{i < TOOLS.length - 1 ? ', ' : '.'}{/each}</p>
<p>
  These need the session’s <strong>DevTools</strong> option: <code>browser_navigate</code>, <code>browser_click</code>,
  <code>browser_type</code>, <code>browser_prepare</code>.
</p>

<Note>
  With DevTools, the agent runs as Chrome’s uid in a container of its own that shares Chrome’s network namespace, so
  it reaches the debugging endpoint on Chrome’s loopback. Its processes are non-dumpable, so Chrome’s uid cannot read
  into them. Without DevTools, Chrome listens on no debugging port.
</Note>

<h2>When a person takes over</h2>
<ol>
  <li>The admin refuses new calls at once and waits up to 5 seconds for the call in flight to end.</li>
  <li>If it does not end, the agent’s container is killed and proven stopped, and it starts again when control comes back.</li>
  <li>From then on, only the person’s input reaches the browser until they give back.</li>
</ol>
