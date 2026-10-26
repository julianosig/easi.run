<script lang="ts">
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
  const far = `# /etc/sysctl.d/90-far-viewers.conf
net.core.default_qdisc = fq
net.ipv4.tcp_congestion_control = bbr
net.ipv4.tcp_slow_start_after_idle = 0`;
  const apply = `echo tcp_bbr | sudo tee /etc/modules-load.d/bbr.conf
sudo modprobe tcp_bbr
sudo sysctl --system`;
  const check = `sysctl net.ipv4.tcp_congestion_control net.core.default_qdisc net.ipv4.tcp_slow_start_after_idle
tc qdisc show
ss -ti | grep -c bbr`;
  const logs = `cd ~/easi.run
docker compose logs -f admin controller
docker compose logs admin | grep -i fingerprint`;
</script>

<h1 tabindex="-1">Operations</h1>
<p class="lead">Updating, removing, reaching the admin from elsewhere, and what restarts do.</p>

<h2>Update</h2>
<p>
  With the installer: the latest one moves your installation to the release it names, pulling that release’s images,
  and removes the images of the release before once the new one runs.
</p>
<Code lang="shell" code={update} />
<p>
  Every run of the installer, with <code>--update</code> or without, also adds Google’s newest Chrome stable from
  Google’s signed repository to the browser image when there is one, and then starts the controller and the admin
  afresh, which ends the sessions running at the time. When Google’s newest Chrome declares other dependencies than the
  release’s browser image was built for, the installer stops and says so, leaving easi.run running as it was: the next
  release’s image follows them.
</p>
<p>
  Installed by hand: in the checkout, pull, rebuild, then start the controller and the admin afresh, so new sessions
  use the new images. Compose reads <code>compose.override.yaml</code> on its own.
</p>
<Code lang="shell" code={byHand} />
<Note tone="warn">
  Restarting the admin stops every running session and ends every login: it removes whatever an earlier run left
  behind before it accepts new sessions.
</Note>

<h2>Remove</h2>
<Code lang="shell" code={remove} />
<p>
  After you confirm (<code>--yes</code> confirms ahead, for an agent), the installer removes what it put on the server:
  easi.run’s containers, with every session, its volumes, networks and images, the <code>docker.service</code> drop-in
  and the firewall rule (unless another easi.run installation on the server still needs them),
  <code>~/.config/easi-run</code>, and Claude Code’s <code>easi</code> server when it is this
  installation’s. It keeps the checkout, <code>~/easi.run</code>, with any <code>.env</code> or override file in it, and Docker’s build cache, which <code>docker builder prune</code> removes for every project on the server.
</p>

<h2>Reaching the admin</h2>
<ul>
  <li>By default the Compose file publishes it on the host’s <code>127.0.0.1:9443</code> only. From another machine, use the command you already use to log in to the server, adding <code>-N -L 9443:127.0.0.1:9443</code> (for example <code>ssh -N -L 9443:127.0.0.1:9443 you@your-server</code>), then open <code>https://localhost:9443</code>.</li>
  <li>Behind your own domain or reverse proxy, set <code>ADMIN_ORIGIN</code> to the exact address people open and add the name to <code>ADMIN_HOSTNAMES</code>, in an override file: <code>compose.yaml</code> sets both itself. Requests from any other origin are refused.</li>
  <li>To use your own certificate, put <code>tls-cert.pem</code> and <code>tls-key.pem</code> in the admin’s state volume.</li>
</ul>
<Code lang="yaml" code={domain} />
<p>
  After you change <code>ADMIN_HOSTNAMES</code>, the next start makes the admin's own certificate again for the new
  names. A certificate you put in the state volume yourself is never replaced.
</p>

<h2>Reach the admin from anywhere</h2>
<p>
  Without an SSH tunnel, you can put one of these in front of the admin, each with your own account. Exposing the admin
  beyond the server is your choice. Either way, set <code>ADMIN_ORIGIN</code> to the address you will open, then run
  <code>bash ~/easi.run/install.sh</code> again. Logins then work at that address only, and a passkey made at another
  hostname does not sign in there.
</p>
<Code lang="yaml" code={origin} />
<h3>Tailscale</h3>
<p>
  On the server, in your tailnet, with HTTPS certificates turned on for it, this serves the admin at
  <code>https://&lt;machine&gt;.&lt;tailnet&gt;.ts.net</code> (that address is your <code>ADMIN_ORIGIN</code>),
  to your tailnet only: <code>serve</code> shares within the tailnet, and Funnel, which publishes to the internet, is
  not used. The admin stays on the server’s <code>127.0.0.1</code>; <code>https+insecure</code> means Tailscale does
  not check the admin’s own certificate on that loopback hop.
</p>
<Code lang="shell" code={tailscale} />
<p><code>sudo tailscale serve reset</code> stops it.</p>
<h3>Cloudflare Tunnel with Cloudflare Access</h3>
<p>
  On a domain in your Cloudflare account, first add a Cloudflare Access application for the name (in Cloudflare One:
  Access controls, Applications, Create new application, Self-hosted and private, with the name as its public
  hostname), with an Allow policy for your own email: Access checks who you are before any request reaches the
  admin’s login page. Then, on the server, create the tunnel and give it the name:
</p>
<Code lang="shell" code={cloudflared} />
<p>
  Save its configuration, with the tunnel’s id that <code>create</code> printed: <code>cloudflared</code> reaches the
  admin on the server’s loopback and checks its certificate against the copy the installer keeps.
</p>
<Code lang="yaml" code={ingress} />
<p>Then run it:</p>
<Code lang="shell" code={run} />
<p>
  What protects the admin then: who may reach the address at all (your tailnet’s devices, or your Access policy),
  then the admin’s sign-in: your password and, when you turned it on, the second step, or a passkey.
</p>

<h2>Viewers far from the server</h2>
<p>
  easi.run sends a viewer only as much of the screen as its connection delivers within its round trip and a little
  more, so a slow or distant connection gets fewer frames rather than late ones. How the server’s TCP then carries
  them is up to its kernel, and Linux’s defaults suit nearby clients. CUBIC, the default, does not pace what it sends
  unless the <code>fq</code> queue does, so what its window allows leaves at the server’s line rate, and a shallow
  queue on a long path drops much of a large burst, after which CUBIC sends far less for a while; and a connection
  idle for a moment starts again from a small window, so an update after a pause takes several round trips. On a
  server far from its viewers (a round trip of 100 ms or more), BBR paces what it sends at about the rate the path
  delivers (with the <code>fq</code> queue, which BBR’s authors recommend, or by itself on recent kernels), and keeping
  the window across pauses lets an update after a pause go at once:
</p>
<Code lang="ini" code={far} />
<Code lang="shell" code={apply} />
<p>
  These settings are the whole host’s, for every TCP connection it makes: apply them on the machine whose connection
  crosses the long path, the server itself when viewers use an SSH tunnel or a published port. The installer does not
  change them. The congestion control applies to connections opened afterwards (reconnect the tunnel and reload the
  viewer), and <code>default_qdisc</code> to interfaces set up afterwards: until the next reboot, an existing
  interface keeps its queue unless you replace it (<code>sudo tc qdisc replace dev eth0 root fq</code>, with your
  interface’s name). To check, the first command shows the three values, the second <code>fq</code> on the
  interface, and the third counts connections using BBR:
</p>
<Code lang="shell" code={check} />
<p>
  To undo it, remove the two files and set the three values back to what the first command showed before; on Debian
  and Ubuntu: <code>sudo sysctl -w net.ipv4.tcp_congestion_control=cubic net.core.default_qdisc=fq_codel
  net.ipv4.tcp_slow_start_after_idle=1</code>. An interface whose queue you replaced keeps <code>fq</code> until the
  next reboot or until you put its queue back (<code>sudo tc qdisc replace dev eth0 root fq_codel</code> on Debian and
  Ubuntu). Then reconnect.
</p>

<h2>A lost second step</h2>
<p>
  Lost the phone with the authenticator app, or the passkey, and the recovery codes too? On the server, this turns
  the second step off; the password alone signs in again at once, without a restart, and you can turn it on again
  under <strong>Security</strong>:
</p>
<Code lang="shell" code={reset} />

<h2>Logs</h2>
<p>
  <code>docker compose</code> finds the installation from its checkout, <code>~/easi.run</code>; from anywhere else,
  <code>docker logs easi-run-admin-1</code> and <code>docker logs easi-run-controller-1</code> show the same.
</p>
<Code lang="shell" code={logs} />
<p>
  The admin logs its certificate’s fingerprint on every start.
</p>
