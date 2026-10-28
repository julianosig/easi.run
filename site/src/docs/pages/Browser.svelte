<script lang="ts">
  import Rich from '../Rich.svelte';
  // The browser container: isolation, the managed policy, per-session flags.
</script>

<h1 tabindex="-1"><Rich message="browser.title" /></h1>
<p class="lead"><Rich message="browser.intro.1" /></p>

<h2><Rich message="browser.isolation.title" /></h2>
<ul>
  <li><Rich message="browser.isolation.1" /></li>
  <li><Rich message="browser.isolation.2" /></li>
  <li><Rich message="browser.isolation.3" /></li>
</ul>

<h2><Rich message="browser.chrome-policy.title" /></h2>
<p><Rich message="browser.chrome-policy.1" /></p>
<ul>
  <li><Rich message="browser.chrome-policy.2" /></li>
  <li><Rich message="browser.chrome-policy.3" /></li>
  <li><Rich message="browser.chrome-policy.4" /></li>
  <li><Rich message="browser.chrome-policy.5" /></li>
</ul>

<h2><Rich message="browser.per-session-flags.title" /></h2>
<table>
  <thead><tr><th><Rich message="browser.per-session-flags.1" /></th><th><Rich message="browser.per-session-flags.2" /></th></tr></thead>
  <tbody>
    <tr><td><Rich message="browser.per-session-flags.3" /></td><td><Rich message="browser.per-session-flags.4" /></td></tr>
    <tr><td><Rich message="browser.per-session-flags.5" /></td><td><Rich message="browser.per-session-flags.6" /></td></tr>
    <tr><td><Rich message="browser.per-session-flags.7" /></td><td><Rich message="browser.per-session-flags.8" /></td></tr>
  </tbody>
</table>

<h2><Rich message="browser.looking-like-an-ordinary.title" /></h2>
<p><Rich message="browser.looking-like-an-ordinary.1" /></p>
