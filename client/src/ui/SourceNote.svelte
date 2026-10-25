<script lang="ts">
  // AGPL-3.0 §13: whoever runs a modified copy offers its source to its users; this link is where they point it.
  let { class: className = '' }: { class?: string } = $props();
</script>

<p class="text-center text-xs text-muted {className}">
  easi.run is free software under the AGPL-3.0 ·
  <a class="font-semibold text-accent hover:underline" href="https://github.com/julianosig/easi.run">Source</a>
</p>
