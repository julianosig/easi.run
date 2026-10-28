<script lang="ts">
  import Rich from '../Rich.svelte';
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  import Vars from '../Vars.svelte';
  import { BENCH } from '../config.ts';
  const admin = `ADMIN_DEMO_SECONDS: "300"
ADMIN_DEMO_SITES: drag=https://drag.play/,typing=https://typing.play/
ADMIN_MAX_SESSIONS: "10"
ADMIN_STATIC_PATH: /demo
ADMIN_FRAME_ANCESTORS: https://easi.run`;
  const controller = `CONTROLLER_INSTANCE: demo
CONTROLLER_BENCH_NETWORK: easi-run-demo-play
CONTROLLER_BENCH_FIXTURE: takeover.play=10.251.0.10
CONTROLLER_BENCH_SPKI_FILE: /run/bench/spki
CONTROLLER_BENCH_FIXTURE_ONLY: "1"
CONTROLLER_MAX_SESSION_SECONDS: "360"`;
  const cert = `demo/sites-cert.sh
DOCKER_GID=$(stat -c %g /var/run/docker.sock) docker compose -f compose.yaml -f demo/compose.yaml up -d --build`;
  const restart = `docker compose -f compose.yaml -f demo/compose.yaml restart controller`;
</script>

<h1 tabindex="-1"><Rich message="demo.title" /></h1>
<p class="lead"><Rich message="demo.intro.1" /></p>

<h2><Rich message="demo.what-changes-in-demo.title" /></h2>
<ul>
  <li><Rich message="demo.what-changes-in-demo.1" /></li>
  <li><Rich message="demo.what-changes-in-demo.2" /></li>
  <li><Rich message="demo.what-changes-in-demo.3" /></li>
  <li><Rich message="demo.what-changes-in-demo.4" /></li>
  <li><Rich message="demo.what-changes-in-demo.5" /></li>
  <li><Rich message="demo.what-changes-in-demo.6" /></li>
</ul>

<h2><Rich message="demo.the-admin.title" /></h2>
<Code lang="yaml" code={admin} />
<p><Rich message="demo.the-admin.1" /></p>
<p><Rich message="demo.the-admin.2" /></p>

<h2><Rich message="demo.the-walled-garden.title" /></h2>
<p><Rich message="demo.the-walled-garden.1" /></p>
<Code lang="yaml" code={controller} />
<p><Rich message="demo.the-walled-garden.2" /></p>
<Code lang="shell" code={cert} />
<p><Rich message="demo.the-walled-garden.3" /></p>
<Code lang="shell" code={restart} />
<Vars vars={BENCH} group="bench" />
<Note tone="warn"><Rich message="demo.the-walled-garden.4" /></Note>
