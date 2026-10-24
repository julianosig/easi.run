<script lang="ts">
  // The browser container: isolation, the managed policy, per-session flags.
</script>

<h1 tabindex="-1">The browser</h1>
<p class="lead">
  Google Chrome as an ordinary, maximized window with its tabs and address bar, in a container of its own.
</p>

<h2>Isolation</h2>
<ul>
  <li>No network of its own: its only way out is the session proxy.</li>
  <li>No capabilities, a read-only root, its own uid, a seccomp profile, and the CPU and memory limits you set.</li>
  <li>A fresh profile per session, kept in memory and removed with the session.</li>
</ul>

<h2>Chrome policy</h2>
<p>A managed policy is built into the image. Among other things it:</p>
<ul>
  <li>blocks <code>chrome://</code> pages (except the new tab page), <code>devtools://</code>, <code>file://</code>, <code>view-source:</code>, <code>javascript:</code> and <code>data:</code> addresses;</li>
  <li>turns off sign-in, sync, the password manager, autofill, printing, extensions, guest mode and new profiles;</li>
  <li>turns off QUIC and Chrome’s own DNS client, so everything goes through the proxy, and sends WebRTC only through it;</li>
  <li>saves downloads to the session’s own downloads folder, which streams them to you.</li>
</ul>

<h2>Per-session flags</h2>
<table>
  <thead><tr><th>Flag</th><th>What it does</th></tr></thead>
  <tbody>
    <tr><td><code>devtools</code></td><td>Opens Chrome’s debugging endpoint on the session’s loopback, for the agent’s browser tools. The agent then runs as Chrome’s uid in Chrome’s network namespace, a weaker separation.</td></tr>
    <tr><td><code>egress</code></td><td>The upstream proxy, by name (see <a href="#/egress">Egress proxies</a>).</td></tr>
    <tr><td><code>kiosk</code></td><td>The page alone on the screen, without tabs or address bar. Only the benchmark profile accepts it; other installations refuse it.</td></tr>
  </tbody>
</table>

<h2>Looking like an ordinary browser</h2>
<p>
  Chrome runs with a real window, real fonts and WebGL on a software renderer, not headless. Its time zone and
  languages follow the installation or the egress.
</p>
