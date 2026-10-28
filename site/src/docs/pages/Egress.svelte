<script lang="ts">
  import Rich from '../Rich.svelte';
  import Code from '../Code.svelte';
  import Note from '../Note.svelte';
  const proxies = `http://user-{session}:secret@proxy.example.net:8080
https://user:secret@proxy.example.net:443
socks5://user:secret@proxy.example.net:1080`;
</script>

<h1 tabindex="-1"><Rich message="egress.title" /></h1>
<p class="lead"><Rich message="egress.intro.1" /></p>

<h2><Rich message="egress.add-a-proxy.title" /></h2>
<p><Rich message="egress.add-a-proxy.1" /></p>
<Code lang="proxy" code={proxies} />
<table>
  <thead><tr><th><Rich message="egress.add-a-proxy.2" /></th><th><Rich message="egress.add-a-proxy.3" /></th></tr></thead>
  <tbody>
    <tr><td><Rich message="egress.add-a-proxy.4" /></td><td><Rich message="egress.add-a-proxy.5" /></td></tr>
    <tr><td><Rich message="egress.add-a-proxy.6" /></td><td><Rich message="egress.add-a-proxy.7" /></td></tr>
    <tr><td><Rich message="egress.add-a-proxy.8" /></td><td><Rich message="egress.add-a-proxy.9" /></td></tr>
    <tr><td><Rich message="egress.add-a-proxy.10" /></td><td><Rich message="egress.add-a-proxy.11" /></td></tr>
  </tbody>
</table>
<p><Rich message="egress.add-a-proxy.12" /></p>

<h2><Rich message="egress.the-check.title" /></h2>
<p><Rich message="egress.the-check.1" /></p>
<p><Rich message="egress.the-check.2" /></p>

<Note tone="warn"><Rich message="egress.the-check.3" /></Note>

<h2><Rich message="egress.what-goes-through-it.title" /></h2>
<p><Rich message="egress.what-goes-through-it.1" /></p>
