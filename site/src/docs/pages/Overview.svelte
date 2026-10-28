<script lang="ts">
  // What easi.run is, what a session gives you, and its parts.
</script>

<h1 tabindex="-1">easi.run</h1>
<p class="lead">
  A real Google Chrome on your own server, for an agent to drive and a person to watch and take over, streamed to
  your browser.
</p>

<p>
  easi.run runs each browser session in its own set of containers and shows it on a web page. The agent works the
  page; you watch it live, and at any moment you <strong>take control</strong>, type a password or a 2FA code
  yourself, and <strong>give back</strong>. Text of four characters or more you type or paste while you hold control
  is replaced by <code>[redacted]</code> in what the agent returns to its caller (see <a href="#/security">Security</a>).
</p>

<h2>What a session gives you</h2>
<ul>
  <li><strong>The picture and the sound:</strong> static content as lossless tiles, motion as AV1 video, sound as Opus.</li>
  <li><strong>Exact control:</strong> only the viewer holding the control lease sends input, and the agent waits while you hold it (see <a href="#/sessions">Sessions</a>).</li>
  <li><strong>Clipboard both ways</strong> (text), <strong>downloads</strong> to your own Downloads folder, and <strong>files</strong> to the page through its file chooser or by dragging them onto the screen.</li>
  <li><strong>Recording</strong> on the server, with your camera and microphone if you want them, straight to your Downloads (see <a href="#/sessions">Sessions</a>).</li>
  <li><strong>An egress per session:</strong> traffic can leave through an upstream proxy you name, with the browser’s time zone and languages set to match.</li>
</ul>

<h2>How it fits together</h2>
<p>
  Two containers run all the time: the <strong>admin</strong>, the HTTPS web app and API you open, and the
  <strong>controller</strong>, the only holder of the Docker socket, which runs a closed set of operations on fixed
  templates: it creates, inspects and stops each session’s containers, attaches to, stops and restarts its agent, and
  removes what an earlier run left. Four more start for each session: the <strong>display</strong>, a headless Wayland compositor that shows
  Chrome, encodes its picture and sound and applies input; the <strong>browser</strong>, Google Chrome; its
  <strong>proxy</strong>, the browser’s only way out; and the <strong>agent</strong>, the Cua Driver, which reads the
  page through its accessibility tree and its screen, and acts through virtual input. <a href="#/security">Security</a> says what each
  may do.
</p>

<p>To view, use a recent Chrome or Chromium. Start with the <a href="#/quick-start">Quick start</a>, or <a href="https://easi.run/#demo">try the live demo</a> first.</p>
