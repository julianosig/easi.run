<script lang="ts">
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  const proxies = `http://user-{session}:secret@proxy.example.net:8080
https://user:secret@proxy.example.net:443
socks5://user:secret@proxy.example.net:1080`;
</script>

<h1 tabindex="-1">Egress proxies</h1>
<p class="lead">
  Let a session’s traffic leave through an upstream proxy you choose, such as a residential or ISP proxy, with the
  browser’s time zone and languages set to match.
</p>

<h2>Add a proxy</h2>
<p>
  In the dashboard, open <strong>Proxies</strong> and add one: a name, and the proxy’s address with its credentials.
  easi.run checks it before it saves it, and saves it only if the check passes. The check learns the country the
  proxy’s traffic leaves from, and the proxy takes that country’s time zone and languages, which you can change.
</p>
<Code lang="proxy" code={proxies} />
<table>
  <thead><tr><th>Field</th><th>Rules</th></tr></thead>
  <tbody>
    <tr><td>Name</td><td>1–32 characters of a–z, 0–9 and -, unique. What people pick when they start a session, and what agents name.</td></tr>
    <tr><td>Proxy</td><td><code>http://</code>, <code>https://</code> or <code>socks5://</code>, then <code>[user[:password]@]host:port</code>. Percent-encode the user and password. With SOCKS5, a user name needs a password, neither empty. <code>{'{session}'}</code> in the user name becomes the session’s id, for providers that keep one exit address per user name.</td></tr>
    <tr><td>Time zone</td><td>An IANA name, such as <code>America/Sao_Paulo</code>. By default, the one the proxy’s country suggests.</td></tr>
    <tr><td>Languages</td><td>1–8 tags such as <code>pt-BR</code>, preferred first. By default, the proxy’s country’s, then <code>en-US</code> and <code>en</code>.</td></tr>
  </tbody>
</table>
<p>Up to 32 proxies. A new address for a proxy is checked the same way, and replaces the old one only if it passes.</p>

<h2>The check</h2>
<p>
  The check runs from where a session’s proxy will reach the upstream, on a network made like a session’s. Through the
  upstream, it reaches the DNS-over-HTTPS resolver every session uses, and fetches Cloudflare’s public trace page
  (<code>https://www.cloudflare.com/cdn-cgi/trace</code>, its certificate verified), which confirms the proxy works and
  says the address its traffic leaves from and that address’s country. The check itself takes at most 10 seconds, and
  says what went wrong when it fails: the proxy cannot be reached, it timed out, it refused the credentials or the
  connection, a TLS certificate did not verify, it does not speak its protocol, or the trace page answered something
  else.
</p>
<p>
  A session starts through a proxy only after a check of the very upstream it will use passes, before any of the
  session’s containers is created; a proxy the same for every session counts a check that passed in the last 30
  seconds. You can check a saved proxy again from the Proxies page at any time.
</p>

<Note tone="warn">
  Proxies’ credentials are kept in the admin’s state volume (<code>egresses.json</code>, readable by the admin only),
  so a backup of that volume holds them. The dashboard never shows them again: each proxy lists its address, whether it
  has a user name and a password, its country, time zone and languages. Only the administrator adds, changes or removes
  proxies; agents start sessions through them by name.
</Note>

<h2>What goes through it</h2>
<p>
  Everything the browser loads, DNS included (over DNS-over-HTTPS through the upstream).
</p>
