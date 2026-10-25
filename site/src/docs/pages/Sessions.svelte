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
    <tr><td><strong>DevTools</strong></td><td>Off by default. On, Chrome’s debugging endpoint listens inside the session, which the agent’s browser tools need. See <a href="#/agent">The agent</a>.</td></tr>
    <tr><td><strong>Egress</strong></td><td>Leave directly, or through one of the deployment’s upstream proxies. The browser then takes that egress’s time zone and languages.</td></tr>
    <tr><td><strong>Profile</strong></td><td>None by default: a fresh browser. A saved profile starts the session with its logins and site data, and the session saves them when you stop it (see Saved profiles below).</td></tr>
  </tbody>
</table>
<p>
  The screen size is set for the whole installation (<code>ADMIN_OUTPUT</code>). The time zone and languages are the
  installation’s (<code>ADMIN_TIMEZONE</code>, <code>ADMIN_LANGUAGES</code>), or, for a session that leaves through an
  egress, that egress’s.
</p>

<h2>Control</h2>
<p>
  One viewer at a time holds the <strong>control lease</strong>; only its input reaches the browser. Taking control
  first pauses the agent’s calls and proves none still runs (see <a href="#/agent">The agent</a>); giving it back lets
  the agent act again.
</p>
<ul>
  <li>Text of four characters or more you type or paste while you hold control is replaced by <code>[redacted]</code> in what the agent returns to its caller (see <a href="#/security">Security</a>).</li>
  <li>Closing the tab, losing the connection or logging out gives control back.</li>
</ul>

<h2>Keyboard and clipboard</h2>
<ul>
  <li>Text travels as text, so what you type does not depend on the server’s keyboard layout.</li>
  <li><strong>Paste</strong> with your usual shortcut: your clipboard’s text goes to the page.</li>
  <li><strong>Copy</strong> on the page while you hold control: a button puts the text on your clipboard, or turn on <strong>Copy what the page copies automatically</strong> to skip it (the public demo copies automatically).</li>
  <li><strong>On a Mac</strong>, Command acts as Control in the remote browser, so ⌘C, ⌘V and ⌘A work as you expect, and Option with an arrow, Backspace or Delete acts as Control too, for word moves and word deletes.</li>
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
  <li>The file streams to your Downloads as it is made, and is complete when you stop. One recording at a time, up to an hour or 8 GiB. Giving control away or closing the tab ends it.</li>
  <li>It records the whole screen, as the person holding control sees it.</li>
</ul>

<h2>Sound</h2>
<p>The page’s sound plays in your browser, with mute and volume controls. Browsers ask for one click before sound can start.</p>

<h2>Saved profiles</h2>
<p>
  Saved profiles keep your logins between sessions, on any site: log in once in a session started from a profile, and
  the next session from it is still logged in. Each is encrypted on your server with a key only your password unlocks.
</p>
<ul>
  <li>Create one under <strong>Saved profiles</strong> on the sessions page, then pick it as the new session’s <strong>Profile</strong>. Its first session starts with a fresh browser.</li>
  <li>Stopping the session saves the profile: Chrome closes cleanly, then its cookies, storage, history and settings are encrypted and kept. Chrome’s caches are left out; it fills them again. Chrome’s password manager stays off: a profile keeps what sites keep in cookies and storage, not your passwords.</li>
  <li>It keeps what Chrome keeps when it closes. A login a site keeps only until the browser closes (one where you did not tick “remember me”, say) ends with the session, as it would when you quit Chrome on a desktop.</li>
  <li>One session at a time uses a profile; a second start from it is refused until the first is stopped.</li>
  <li>Agents use only the profiles you allow them: under <strong>Agents</strong> on a profile, tick the tokens (and the local MCP clients) that may start sessions from it. To any other, the profile does not exist.</li>
  <li>After the server restarts, the profiles stay locked until you log in with your password. Signed in with a passkey, enter your password once under <strong>Unlock saved profiles</strong>.</li>
  <li>A profile holds up to 512 MiB. A save that cannot be made (a larger profile, or Chrome not closing within 20 seconds) keeps the last save, and the profile says why. A session the server’s restart ends keeps the last save too.</li>
  <li>Up to 16 profiles. Deleting one deletes its saved data for good. The public demo has none.</li>
</ul>

<h2>Closing Chrome</h2>
<p>
  Closing Chrome’s last window, or quitting it, opens it again at the session’s first page, with the same cookies, up
  to 50 times per session.
</p>

<Note>
  Without a saved profile, a session keeps nothing: it starts with a fresh browser profile, and stopping it removes
  its containers and their data.
</Note>
