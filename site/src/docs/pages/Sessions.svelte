<script lang="ts">
  import Note from '../Note.svelte';
</script>

<h1 tabindex="-1">Sessions</h1>
<p class="lead">One session is one Chrome, its screen, its sound, and the agent that drives it.</p>

<h2>Starting a session</h2>
<table>
  <thead><tr><th>Option</th><th>What it does</th></tr></thead>
  <tbody>
    <tr><td><strong>URL</strong></td><td>The first page, <code>https://</code> or <code>http://</code>, up to 2048 bytes.</td></tr>
    <tr><td><strong>DevTools</strong></td><td>Off by default. On, Chrome’s debugging endpoint listens inside the session, which the agent’s browser tools need. See <a href="#/agent">The agent</a> for what it costs.</td></tr>
    <tr><td><strong>Egress</strong></td><td>Leave directly, or through one of the deployment’s upstream proxies. The browser then takes that egress’s time zone and languages.</td></tr>
  </tbody>
</table>
<p>
  The screen size, time zone and languages are set for the whole installation (<code>ADMIN_OUTPUT</code>,
  <code>ADMIN_TIMEZONE</code>, <code>ADMIN_LANGUAGES</code>) or by the egress, never per session.
</p>

<h2>Control</h2>
<p>
  One viewer at a time holds the <strong>control lease</strong>; only its input reaches the browser. Taking control
  first stops the agent and proves it stopped. Giving it back lets the agent act again.
</p>
<ul>
  <li>What you type or paste while you hold control (4+ characters, unchanged) reaches the agent’s later observations as <code>[redacted]</code>.</li>
  <li>Closing the tab, losing the connection or logging out gives control back.</li>
</ul>

<h2>Keyboard and clipboard</h2>
<ul>
  <li>Text travels as text, so what you type does not depend on the server’s keyboard layout.</li>
  <li><strong>Paste</strong> with your usual shortcut: your clipboard’s text goes to the page.</li>
  <li><strong>Copy</strong> on the page while you hold control: a button puts the text on your clipboard, or turn on <strong>Copy what the page copies automatically</strong> to skip it (the public demo always copies automatically).</li>
  <li><strong>On a Mac</strong>, Command acts as Control in the remote browser, so ⌘C, ⌘V and ⌘A work as you expect.</li>
</ul>

<h2>Files and downloads</h2>
<ul>
  <li><strong>Upload:</strong> when the page opens a file chooser, your own file picker opens; or drag files onto the screen.</li>
  <li><strong>Download:</strong> the page’s downloads stream to your own browser’s Downloads.</li>
</ul>

<h2>Recording</h2>
<ul>
  <li>Whoever holds control can record the session: press <strong>Record</strong>, pick a microphone and a camera (or none), the camera’s corner radius and border, and whether to include the page’s sound. The camera is a square: drag it to place it, and drag its corner handle to resize it.</li>
  <li>The display records the screen itself, at its own resolution: AV1 video at 30 frames a second (10 Mbit/s) and Opus sound at 128 kbit/s, in a <code>.webm</code> file.</li>
  <li>Your camera and microphone are sent to the server while it records, and drawn and mixed into the video there.</li>
  <li>The file streams to your Downloads as it is made, and is complete when you stop. One recording at a time, up to an hour. Giving control away or closing the tab ends it.</li>
  <li>It records the whole screen, including what the agent receives as <code>[redacted]</code>.</li>
</ul>

<h2>Sound</h2>
<p>The page’s sound plays in your browser, with mute and volume controls. Browsers ask for one click before sound can start.</p>

<h2>Closing Chrome</h2>
<p>
  Closing Chrome’s last window, or quitting it, opens it again at the session’s first page, with the same cookies, up
  to 50 times per session. After a Chrome crash the session has no browser any more: stop it and start a new one.
</p>

<Note>
  Sessions do not keep anything: each one starts with a fresh browser profile, and stopping it removes its containers
  and their data.
</Note>
