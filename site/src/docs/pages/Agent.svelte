<script lang="ts">
  import Rich from '../Rich.svelte';
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  import { TOOLS } from '../tools.ts';
  const call = `BASE=https://localhost:9443
# Log in once: keep the cookie and the CSRF token it returns.
LOGIN=$(curl -sk -c jar -H "Origin: $BASE" -H 'Content-Type: application/json' \\
  -d '{"password":"your password"}' $BASE/api/login)
# With the second step on, the password asks for a code: the authenticator app's, or a recovery code.
STEP=$(echo "$LOGIN" | jq -r '.second_step.step // empty')
if [ -n "$STEP" ]; then
  read -rp 'Code: ' CODE
  LOGIN=$(curl -sk -c jar -H "Origin: $BASE" -H 'Content-Type: application/json' \\
    -d "{\\"step\\":\\"$STEP\\",\\"code\\":\\"$CODE\\"}" $BASE/api/login/code)
fi
CSRF=$(echo "$LOGIN" | jq -r .csrf)

# One tool call on session 0123456789ab.
curl -sk -b jar -H "Origin: $BASE" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \\
  -d '{"tool":"get_desktop_state","arguments":{}}' \\
  $BASE/api/sessions/0123456789ab/agent`;
</script>

<h1 tabindex="-1"><Rich message="agent.title" /></h1>
<p class="lead"><Rich message="agent.intro.1" /></p>

<h2><Rich message="agent.how-you-reach-it.title" /></h2>
<p><Rich message="agent.how-you-reach-it.1" /></p>
<Code lang="shell" code={call} />
<p><Rich message="agent.how-you-reach-it.2" /></p>
<ul>
  <li><Rich message="agent.how-you-reach-it.3" /></li>
  <li><Rich message="agent.how-you-reach-it.4" /></li>
  <li><Rich message="agent.how-you-reach-it.5" /></li>
  <li><Rich message="agent.how-you-reach-it.6" /></li>
  <li><Rich message="agent.how-you-reach-it.7" /></li>
</ul>

<h2><Rich message="agent.tools.title" /></h2>
<p><Rich message="agent.tools.1" /></p>
<p>{#each TOOLS as t, i (t)}<code>{t}</code>{i < TOOLS.length - 1 ? ', ' : '.'}{/each}</p>
<p><Rich message="agent.tools.2" /></p>

<Note><Rich message="agent.tools.3" /></Note>

<h2><Rich message="agent.when-a-person-takes.title" /></h2>
<p><Rich message="agent.when-a-person-takes.1" /></p>
<ol>
  <li><Rich message="agent.when-a-person-takes.2" /></li>
  <li><Rich message="agent.when-a-person-takes.3" /></li>
  <li><Rich message="agent.when-a-person-takes.4" /></li>
</ol>
