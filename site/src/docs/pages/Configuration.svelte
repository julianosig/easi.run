<script lang="ts">
  import Rich from '../Rich.svelte';
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  import Vars from '../Vars.svelte';
  import { ADMIN, COMPOSE, CONTROLLER } from '../config.ts';
  const env = `# ~/easi.run/.env, next to compose.yaml
ADMIN_MAX_SESSIONS=8
BROWSER_CPUS=1
BROWSER_MEMORY_MIB=1024
ADMIN_TIMEZONE=America/Sao_Paulo
ADMIN_LANGUAGES=pt-BR,pt,en-US,en`;
  const override = `# compose.override.yaml: anything the Compose file does not expose
services:
  admin:
    environment:
      ADMIN_OUTPUT: 1600x900`;
  const logo = `# compose.override.yaml, next to compose.yaml: ~/easi.run/logo.png is your logo,
# and .env holds DISPLAY_LOGO_FILE=/etc/controller/logo.png
services:
  controller:
    volumes:
      - ./logo.png:/etc/controller/logo.png:ro`;
</script>

<h1 tabindex="-1"><Rich message="configuration.title" /></h1>
<p class="lead"><Rich message="configuration.intro.1" /></p>

<h2><Rich message="configuration.compose-variables.title" /></h2>
<p><Rich message="configuration.compose-variables.1" /></p>
<p><Rich message="configuration.compose-variables.2" /></p>
<Vars vars={COMPOSE} group="compose" />
<Code lang=".env" code={env} />

<Note><Rich message="configuration.compose-variables.3" /></Note>
<Code lang="yaml" code={override} />

<h2><Rich message="configuration.the-admin.title" /></h2>
<Vars vars={ADMIN} group="admin" />

<h2><Rich message="configuration.the-controller.title" /></h2>
<Vars vars={CONTROLLER} group="controller" />
<p><Rich message="configuration.the-controller.logo" /></p>
<Code lang="yaml" code={logo} />

<h2><Rich message="configuration.session-containers.title" /></h2>
<p><Rich message="configuration.session-containers.1" /></p>

<h2><Rich message="configuration.running-more-than-one.title" /></h2>
<p><Rich message="configuration.running-more-than-one.1" /></p>
