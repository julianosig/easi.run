<script lang="ts">
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  const file = `{
  "egresses": [
    {
      "name": "br-home",
      "proxy": "http://user-{session}:secret@proxy.example.net:8080",
      "timezone": "America/Sao_Paulo",
      "languages": ["pt-BR", "pt", "en-US", "en"]
    }
  ]
}`;
  const run = `sudo chown 10004 egress.json && sudo chmod 0400 egress.json
DOCKER_GID=$(stat -c %g /var/run/docker.sock) \\
  docker compose -f compose.yaml -f compose.egress.yaml up -d`;
</script>

<h1 tabindex="-1">Egress proxies</h1>
<p class="lead">
  Let a session’s traffic leave through an upstream proxy you choose, such as a residential or ISP proxy, with the
  browser’s time zone and languages set to match.
</p>

<h2>The egress file</h2>
<Code lang="egress.json" code={file} />
<table>
  <thead><tr><th>Field</th><th>Rules</th></tr></thead>
  <tbody>
    <tr><td><code>name</code></td><td>1–32 characters of a–z, 0–9 and -, unique. What people pick when they start a session.</td></tr>
    <tr><td><code>proxy</code></td><td><code>http://</code>, <code>https://</code> or <code>socks5://</code>, then <code>[user[:password]@]host:port</code>. Percent-encode the user and password. SOCKS5 needs both. <code>{'{session}'}</code> in the user name becomes the session’s id, for providers that keep one exit address per user name.</td></tr>
    <tr><td><code>timezone</code></td><td>An IANA name. Where the proxy leaves: easi.run cannot check it.</td></tr>
    <tr><td><code>languages</code></td><td>1–8 tags such as <code>pt-BR</code>, preferred first.</td></tr>
  </tbody>
</table>
<p>Up to 32 egresses, 64 KiB. Unknown fields are refused, and a malformed file stops the admin.</p>

<h2>Turn it on</h2>
<Code lang="shell" code={run} />
<Note tone="warn">
  The file holds the proxies’ credentials. The admin refuses it unless it belongs to the admin’s uid (10004) or root,
  its group may at most read it, and others have no access. People see each egress’s name, time zone and languages,
  never its address or credentials.
</Note>

<h2>What goes through it</h2>
<p>
  Everything the browser loads, DNS included (over DNS-over-HTTPS through the upstream). The session proxy still
  refuses non-public addresses and allows only ports 80 and 443.
</p>
