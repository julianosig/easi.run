<script lang="ts">
  // What easi.run is, what a session gives you, and its parts.
</script>

<h1 tabindex="-1">easi.run</h1>
<p class="lead">
  A real Google Chrome on your own server, for an agent to drive and a person to watch and take over, streamed to
  any web browser.
</p>

<p>
  easi.run runs each browser session in its own set of containers and shows it on a web page. The agent works the
  page; you watch it live, and at any moment you <strong>take control</strong>, type a password or a 2FA code
  yourself, and <strong>give back</strong>. What you type while you hold control (4+ characters, unchanged) reaches the agent only
  as <code>[redacted]</code>.
</p>

<h2>What a session gives you</h2>
<ul>
  <li><strong>The picture and the sound:</strong> static content as lossless tiles, motion as AV1 video, sound as Opus.</li>
  <li><strong>Exact control:</strong> only the viewer holding the control lease sends input; a takeover stops the agent first.</li>
  <li><strong>Clipboard both ways</strong> (text), <strong>downloads</strong> to your own Downloads folder, and <strong>files</strong> to the page through its file chooser or by dragging them onto the screen.</li>
  <li><strong>Recording</strong> on the server at full quality (AV1 and Opus, WebM), with your camera and microphone if you want them, straight to your Downloads.</li>
  <li><strong>An egress per session:</strong> traffic can leave through an upstream proxy you name, with the browser’s time zone and languages set to match.</li>
</ul>

<h2>How it fits together</h2>
<p>Two containers run all the time; four more start for each session.</p>
<table>
  <thead><tr><th>Part</th><th>What it does</th></tr></thead>
  <tbody>
    <tr><td><strong>admin</strong></td><td>The HTTPS web app and API you open: logins, sessions, the viewer streams, the agent’s tool calls.</td></tr>
    <tr><td><strong>controller</strong></td><td>The only holder of the Docker socket. Creates and removes each session’s containers from fixed templates, nothing else.</td></tr>
    <tr><td><strong>display</strong> (per session)</td><td>A headless Wayland compositor: shows Chrome, encodes the picture and sound, applies input.</td></tr>
    <tr><td><strong>browser</strong> (per session)</td><td>Google Chrome with no network of its own, no capabilities and a read-only root.</td></tr>
    <tr><td><strong>proxy</strong> (per session)</td><td>The browser’s only way out: public addresses only, optionally through your upstream proxy.</td></tr>
    <tr><td><strong>agent</strong> (per session)</td><td>The Cua Driver, which reads the page through the accessibility tree and acts through virtual input.</td></tr>
  </tbody>
</table>

<h2>It runs in Docker</h2>
<p>
  easi.run needs Docker: every part of it runs in a container, and each session gets containers of its own. They are
  hardened: no capabilities, read-only roots, by default a uid per role, no shell, and limits on CPU and memory. The
  browser has no direct network; its only way out is the session’s proxy, which refuses your private network. See
  <a href="#/security">Security</a> for each boundary.
</p>

<h2>Requirements</h2>
<ul>
  <li>Linux on x86-64 with Docker Engine 28 or newer: easi.run does not run without Docker.</li>
  <li>To view: a recent Chrome or Chromium (the client is tested with Chrome).</li>
</ul>
<p>Start with the <a href="#/quick-start">Quick start</a>, or <a href="https://easi.run/#demo">try the live demo</a> first.</p>
