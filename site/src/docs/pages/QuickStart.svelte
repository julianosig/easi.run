<script lang="ts">
  import Rich from '../Rich.svelte';
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  import { NAME, ORIGIN, REPO } from '../../lib/site.ts';
  const prompt = `Read ${ORIGIN}/install.md and install ${NAME} on this server.`;
  const install = `curl -fsSLO ${ORIGIN}/install.sh && less install.sh && bash install.sh`;
  const tunnel = 'ssh -N -L 9443:127.0.0.1:9443 you@your-server';
  const run = `git clone ${REPO} && cd easi.run
DOCKER_GID=$(stat -c %g /var/run/docker.sock) docker compose up -d --build`;
  const firewall = 'sudo iptables -t raw -I PREROUTING 1 -i er-+ -m addrtype --dst-type LOCAL -j DROP';
</script>

<h1 tabindex="-1"><Rich message="quick-start.title" /></h1>
<p class="lead"><Rich message="quick-start.intro.1" /></p>

<h2><Rich message="quick-start.1-install-it.title" /></h2>
<p><Rich message="quick-start.1-install-it.1" /></p>
<Code lang="prompt" code={prompt} />
<p><Rich message="quick-start.1-install-it.2" /></p>
<Code lang="shell" code={install} />
<p><Rich message="quick-start.1-install-it.3" /></p>
<ol>
  <li><Rich message="quick-start.1-install-it.4" /></li>
  <li><Rich message="quick-start.1-install-it.5" /></li>
  <li><Rich message="quick-start.1-install-it.6" /></li>
  <li><Rich message="quick-start.1-install-it.7" /></li>
  <li><Rich message="quick-start.1-install-it.8" /></li>
  <li><Rich message="quick-start.1-install-it.9" /></li>
  <li><Rich message="quick-start.1-install-it.10" /></li>
  <li><Rich message="quick-start.1-install-it.11" /></li>
</ol>
<p><Rich message="quick-start.1-install-it.12" /></p>

<h2><Rich message="quick-start.2-open-it.title" /></h2>
<p><Rich message="quick-start.2-open-it.1" /></p>
<Code lang="shell" code={tunnel} />
<p><Rich message="quick-start.2-open-it.2" /></p>
<p><Rich message="quick-start.2-open-it.3" /></p>

<h2><Rich message="quick-start.3-create-your-password.title" /></h2>
<ol>
  <li><Rich message="quick-start.3-create-your-password.1" /></li>
  <li><Rich message="quick-start.3-create-your-password.2" /></li>
  <li><Rich message="quick-start.3-create-your-password.3" /></li>
  <li><Rich message="quick-start.3-create-your-password.4" /></li>
</ol>

<h2><Rich message="quick-start.without-the-installer.title" /></h2>
<p><Rich message="quick-start.without-the-installer.1" /></p>
<Code lang="shell" code={firewall} />
<p><Rich message="quick-start.without-the-installer.2" /></p>
<Code lang="shell" code={run} />
<p><Rich message="quick-start.without-the-installer.3" /></p>

<Note><Rich message="quick-start.without-the-installer.4" /></Note>
