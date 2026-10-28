<script lang="ts">
  import Rich from '../Rich.svelte';
  import { REPO } from '../../lib/site.ts';
</script>

<h1 tabindex="-1"><Rich message="security.title" /></h1>
<p class="lead"><Rich message="security.intro.1" /></p>

<h2><Rich message="security.containers-and-what-each.title" /></h2>
<table>
  <thead><tr><th><Rich message="security.containers-and-what-each.1" /></th><th><Rich message="security.containers-and-what-each.2" /></th><th><Rich message="security.containers-and-what-each.3" /></th></tr></thead>
  <tbody>
    <tr><td><Rich message="security.containers-and-what-each.4" /></td><td><Rich message="security.containers-and-what-each.5" /></td><td><Rich message="security.containers-and-what-each.6" /></td></tr>
    <tr><td><Rich message="security.containers-and-what-each.7" /></td><td><Rich message="security.containers-and-what-each.8" /></td><td><Rich message="security.containers-and-what-each.9" /></td></tr>
    <tr><td><Rich message="security.containers-and-what-each.10" /></td><td><Rich message="security.containers-and-what-each.11" /></td><td><Rich message="security.containers-and-what-each.12" /></td></tr>
    <tr><td><Rich message="security.containers-and-what-each.13" /></td><td><Rich message="security.containers-and-what-each.14" /></td><td><Rich message="security.containers-and-what-each.15" /></td></tr>
    <tr><td><Rich message="security.containers-and-what-each.16" /></td><td><Rich message="security.containers-and-what-each.17" /></td><td><Rich message="security.containers-and-what-each.18" /></td></tr>
    <tr><td><Rich message="security.containers-and-what-each.19" /></td><td><Rich message="security.containers-and-what-each.20" /></td><td><Rich message="security.containers-and-what-each.21" /></td></tr>
  </tbody>
</table>
<ul>
  <li><Rich message="security.containers-and-what-each.22" /></li>
  <li><Rich message="security.containers-and-what-each.23" /></li>
</ul>

<h2><Rich message="security.network.title" /></h2>
<ul>
  <li><Rich message="security.network.1" /></li>
  <li><Rich message="security.network.2" /></li>
  <li><Rich message="security.network.3" /></li>
  <li><Rich message="security.network.4" /></li>
</ul>

<h2><Rich message="security.the-admin.title" /></h2>
<ul>
  <li><Rich message="security.the-admin.1" /></li>
  <li><Rich message="security.the-admin.2" /></li>
  <li><Rich message="security.the-admin.3" /></li>
  <li><Rich message="security.the-admin.4" /></li>
  <li><Rich message="security.the-admin.5" /></li>
</ul>
<p><Rich message="security.the-admin.6" params={{ href1: `${REPO}/blob/main/docs/security.md#boundaries` }} /></p>

<h2><Rich message="security.control-and-redaction.title" /></h2>
<ul>
  <li><Rich message="security.control-and-redaction.1" /></li>
  <li id="redaction"><Rich message="security.control-and-redaction.2" /></li>
  <li><Rich message="security.control-and-redaction.3" /></li>
</ul>

<h2><Rich message="security.what-is-kept.title" /></h2>
<ul>
  <li><Rich message="security.what-is-kept.1" /></li>
  <li><Rich message="security.what-is-kept.2" /></li>
  <li><Rich message="security.what-is-kept.3" /></li>
  <li><Rich message="security.what-is-kept.4" /></li>
</ul>

<h2><Rich message="security.persistent-sessions.title" /></h2>
<ul>
  <li><Rich message="security.persistent-sessions.1" /></li>
  <li><Rich message="security.persistent-sessions.2" params={{ href1: `${REPO}/blob/main/docs/security.md#boundaries` }} /></li>
  <li><Rich message="security.persistent-sessions.3" /></li>
  <li><Rich message="security.persistent-sessions.4" /></li>
</ul>

<p><Rich message="security.persistent-sessions.5" params={{ href1: `${REPO}/blob/main/docs/security.md`, href2: `${REPO}/blob/main/SECURITY.md` }} /></p>
