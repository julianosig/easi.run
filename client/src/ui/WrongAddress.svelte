<script lang="ts">
  // The admin refused a request because this page is open at another address than the one it is set up for (see
  // `wrongOrigin`): its login and every change belong to that one address, so the page links there, carrying `href`.
  let { origin, href }: { origin: string; href: string } = $props();
</script>

<p class="mt-3 font-semibold text-bad" role="alert">
  easi.run is set up for <a class="text-accent underline" {href}>{origin}</a>, not this address: open it there, then try again.
</p>
