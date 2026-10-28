<script lang="ts">
  import Rich from '../Rich.svelte';
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  const update = 'curl -fsSLO https://easi.run/install.sh && bash install.sh --update';
  const byHand = `cd ~/easi.run && git pull
export DOCKER_GID=$(stat -c %g /var/run/docker.sock)
docker compose up -d --build
docker compose up -d --no-deps --force-recreate controller admin`;
  const remove = 'bash ~/easi.run/install.sh --uninstall';
  const reset = 'docker exec easi-run-admin-1 admin reset-second-step';
  const domain = `# compose.override.yaml, next to compose.yaml
services:
  admin:
    environment:
      ADMIN_ORIGIN: https://browser.example.com
      ADMIN_HOSTNAMES: browser.example.com,localhost,127.0.0.1`;
  const origin = `# compose.override.yaml, next to compose.yaml
services:
  admin:
    environment:
      ADMIN_ORIGIN: https://browser.example.com`;
  const tailscale = 'sudo tailscale serve --bg https+insecure://localhost:9443';
  const cloudflared = `cloudflared tunnel login
cloudflared tunnel create easi
cloudflared tunnel route dns easi browser.example.com`;
  const run = 'cloudflared tunnel --config ~/.cloudflared/easi.yml run easi';
  const ingress = `# ~/.cloudflared/easi.yml
tunnel: <tunnel-id>
credentials-file: /home/you/.cloudflared/<tunnel-id>.json
ingress:
  - hostname: browser.example.com
    service: https://localhost:9443
    originRequest:
      originServerName: localhost
      caPool: /home/you/.config/easi-run/tls-cert.pem
  - service: http_status:404`;
  const far = `# /etc/sysctl.d/90-easi-run.conf
net.core.default_qdisc=fq
net.ipv4.tcp_congestion_control=bbr
net.ipv4.tcp_slow_start_after_idle=0`;
  const udpEnv = `# ~/easi.run/.env
ADMIN_UDP_URL=https://100.101.102.103:9443`;
  const check = `sysctl net.ipv4.tcp_congestion_control net.core.default_qdisc net.ipv4.tcp_slow_start_after_idle
tc qdisc show
ss -ti | grep -c bbr`;
  const logs = `cd ~/easi.run
docker compose logs -f admin controller
docker compose logs admin | grep -i fingerprint`;
</script>

<h1 tabindex="-1"><Rich message="operations.title" /></h1>
<p class="lead"><Rich message="operations.intro.1" /></p>

<h2><Rich message="operations.update.title" /></h2>
<p><Rich message="operations.update.1" /></p>
<Code lang="shell" code={update} />
<p><Rich message="operations.update.2" /></p>
<p><Rich message="operations.update.3" /></p>
<Code lang="shell" code={byHand} />
<Note tone="warn"><Rich message="operations.update.4" /></Note>

<h2><Rich message="operations.remove.title" /></h2>
<Code lang="shell" code={remove} />
<p><Rich message="operations.remove.1" /></p>

<h2><Rich message="operations.reaching-the-admin.title" /></h2>
<ul>
  <li><Rich message="operations.reaching-the-admin.1" /></li>
  <li><Rich message="operations.reaching-the-admin.2" /></li>
  <li><Rich message="operations.reaching-the-admin.3" /></li>
</ul>
<Code lang="yaml" code={domain} />
<p><Rich message="operations.reaching-the-admin.4" /></p>

<h2><Rich message="operations.reach-the-admin-from.title" /></h2>
<p><Rich message="operations.reach-the-admin-from.1" /></p>
<Code lang="yaml" code={origin} />
<h3><Rich message="operations.tailscale.title" /></h3>
<p><Rich message="operations.tailscale.1" /></p>
<Code lang="shell" code={tailscale} />
<p><Rich message="operations.tailscale.2" /></p>
<h3><Rich message="operations.cloudflare-tunnel-with-cloudflare.title" /></h3>
<p><Rich message="operations.cloudflare-tunnel-with-cloudflare.1" /></p>
<Code lang="shell" code={cloudflared} />
<p><Rich message="operations.cloudflare-tunnel-with-cloudflare.2" /></p>
<Code lang="yaml" code={ingress} />
<p><Rich message="operations.cloudflare-tunnel-with-cloudflare.3" /></p>
<Code lang="shell" code={run} />
<p><Rich message="operations.cloudflare-tunnel-with-cloudflare.4" /></p>

<h2><Rich message="operations.viewers-far-from-the.title" /></h2>
<p><Rich message="operations.viewers-far-from-the.1" /></p>
<h3><Rich message="operations.the-media-over-udp.title" /></h3>
<p><Rich message="operations.the-media-over-udp.1" /></p>
<Code lang="ini" code={udpEnv} />
<h3><Rich message="operations.the-server-s-tcp.title" /></h3>
<p><Rich message="operations.the-server-s-tcp.1" /></p>
<Code lang="ini" code={far} />
<p><Rich message="operations.the-server-s-tcp.2" /></p>
<Code lang="shell" code={check} />

<h2><Rich message="operations.a-lost-second-step.title" /></h2>
<p><Rich message="operations.a-lost-second-step.1" /></p>
<Code lang="shell" code={reset} />

<h2><Rich message="operations.logs.title" /></h2>
<p><Rich message="operations.logs.1" /></p>
<Code lang="shell" code={logs} />
<p><Rich message="operations.logs.2" /></p>
