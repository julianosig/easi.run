<script lang="ts">
  // A QR code for a phone's camera (see `qrPath`): dark on white whatever the page's colors, at a size it reads well.
  import { qrPath } from './qr.ts';

  let { text, label }: { text: string; label: string } = $props();
  const qr = $derived(qrPath(text));
</script>

<svg class="h-auto w-full max-w-56" viewBox="0 0 {qr.size} {qr.size}" shape-rendering="crispEdges" role="img" aria-label={label}>
  <rect width={qr.size} height={qr.size} fill="#ffffff" />
  <path d={qr.path} fill="#000000" />
</svg>
