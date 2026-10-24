<script lang="ts">
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  import Vars from '../Vars.svelte';
  import { ADMIN, COMPOSE, CONTROLLER } from '../config.ts';
  const env = `# .env, next to compose.yaml
DOCKER_GID=998
BROWSER_CPUS=1
BROWSER_MEMORY_MIB=1024
ADMIN_TIMEZONE=America/Sao_Paulo
ADMIN_LANGUAGES=pt-BR,pt,en-US,en`;
  const override = `# compose.override.yaml: anything the Compose file does not expose
services:
  admin:
    environment:
      ADMIN_MAX_SESSIONS: "8"
      ADMIN_OUTPUT: 1600x900`;
</script>

<h1 tabindex="-1">Configuration</h1>
<p class="lead">
  Everything is set with environment variables, read once at start. There are two layers: the Compose variables you
  set in a <code>.env</code> file, and the containers’ own environment.
</p>

<h2>Compose variables</h2>
<p>
  <code>compose.yaml</code> reads these from your shell or a <code>.env</code> file next to it and passes them on. Run
  <code>docker compose up -d</code> again after a change.
</p>
<Vars vars={COMPOSE} />
<Code lang=".env" code={env} />

<Note>
  Other <code>ADMIN_*</code> or <code>CONTROLLER_*</code> names in <code>.env</code> do nothing on their own: Compose
  passes only what the file names. Set them in an override file instead.
</Note>
<Code lang="yaml" code={override} />

<h2>The admin</h2>
<p>Restarting the admin ends every login and stops every running session.</p>
<Vars vars={ADMIN} />

<h2>The controller</h2>
<p>
  The controller resolves image tags once, when it starts: restart it after rebuilding a session image
  (<code>docker compose restart controller</code>).
</p>
<Vars vars={CONTROLLER} />

<h2>Session containers</h2>
<p>
  The display, browser, proxy and agent containers get their environment from the controller’s fixed templates; you do
  not set it. The browser’s CPU and memory come from the controller’s variables above; the display, proxy and agent
  limits are fixed. The walled-garden settings are under
  <a href="#/demo">Public demo</a>.
</p>

<h2>Running more than one installation</h2>
<p>
  Give each installation its own <code>ADMIN_PORT</code> and <code>CONTROLLER_INSTANCE</code>, and its own state,
  controller and streams volumes (<code>STREAMS_VOLUME</code> names one explicitly, so a different Compose project name
  alone is not enough).
</p>
