<script lang="ts">
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
DOCKER_GID=$(stat -c %g /var/run/docker.sock) docker compose -f demo/compose.yaml up -d --build`;
  const restart = `docker compose -f demo/compose.yaml restart controller`;
</script>

<h1 tabindex="-1">Public demo</h1>
<p class="lead">
  The mode behind the live demo on <a href="https://easi.run/#demo">easi.run</a>: anyone gets a short turn with a real browser that
  can reach only your demo pages.
</p>

<h2>What changes in demo mode</h2>
<ul>
  <li>No password: opening the demo starts a visit at once, at the first site of <code>ADMIN_DEMO_SITES</code>.</li>
  <li>Up to <code>ADMIN_MAX_SESSIONS</code> visits run at once, one per client: an IPv4 address, or an IPv6 /64 network, as Cloudflare’s tunnel reports it. Anyone else waits, and starts as soon as a place is free.</li>
  <li>A visit ends at its deadline, whatever happens, or sooner once nobody has watched its browser for 30 seconds; its place is free once its session is confirmed removed.</li>
  <li>Visitors can switch between the demos during their visit, and copy and paste go straight to their clipboard.</li>
  <li>A recording stops by itself 10 seconds before the visit ends, so its download finishes in time.</li>
  <li>Everything an administrator does (more sessions, egresses, settings, agent calls) is refused.</li>
</ul>

<h2>The admin</h2>
<Code lang="yaml" code={admin} />
<p>
  Demo mode uses no egress (the admin refuses to start in it with any egress configured), and the admin must be reachable only through the tunnel: it trusts the tunnel’s
  <code>CF-Connecting-IP</code> header to tell clients apart.
</p>
<p>
  To show the demo inside your own page, as easi.run’s landing does, name that page’s origin in
  <code>ADMIN_FRAME_ANCESTORS</code>; nothing else may frame it. <code>ADMIN_STATIC_PATH</code> serves the web client
  under its own path, so the demo and your page can share one host.
</p>

<h2>The walled garden</h2>
<p>
  The controller sends every address a demo browser asks for to one server you run on an internal Docker network, so
  no visitor reaches the internet. It refuses to start the walled garden on a network that is not internal.
</p>
<Code lang="yaml" code={controller} />
<p>
  That server shows one self-signed certificate for every name, which Chrome accepts by its pinned key: the file
  <code>CONTROLLER_BENCH_SPKI_FILE</code> names holds the SHA-256 of that public key. easi.run’s own demo
  (<code>demo/compose.yaml</code>) makes the certificate, its key, and that hash once, with
  <code>demo/sites-cert.sh</code>, before its first start:
</p>
<Code lang="shell" code={cert} />
<p>
  The controller resolves the session images’ tags once, when it starts. After rebuilding a session image, restart it;
  this ends the visits under way:
</p>
<Code lang="shell" code={restart} />
<Vars vars={BENCH} />
<Note tone="warn">
  These settings change what a session may reach and which certificate Chrome accepts. Use them for a demo or a
  benchmark, never on an installation people use for real browsing.
</Note>
