<script lang="ts">
  // The recovery codes, shown this once: copy or download them, and say they are saved before going on.
  import { copyText } from './api.ts';
  import { recoveryFile } from './second.ts';
  import { button, primary } from './ui.ts';

  let { codes, onDone }: { codes: string[]; onDone: () => void } = $props();
  let saved = $state(false);
  let copied = $state(false);
  let error = $state('');
  const text = $derived(recoveryFile(codes, location.origin));

  async function copy() {
    if (await copyText(text)) {
      copied = true;
      error = '';
    } else error = 'Copying was refused; download them, or select them and copy them yourself.';
  }

  function download() {
    const url = URL.createObjectURL(new Blob([text], { type: 'text/plain' }));
    const link = Object.assign(document.createElement('a'), { href: url, download: 'easi-run-recovery-codes.txt' });
    link.click();
    // Once the browser took the download.
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
</script>

<h2 class="text-xl font-semibold tracking-[-0.02em]">Save your recovery codes</h2>
<p class="mt-1 text-muted">
  If you lose your phone or passkey, sign in with your password, then enter one of these codes in place of the second
  step; each works once. This is the only time they are shown, so keep them somewhere safe, such as your password
  manager.
</p>
<ul class="mt-4 grid select-all gap-x-4 sm:grid-cols-2 gap-y-1.5 rounded-xl border border-line bg-wash p-4 font-mono text-[15px]">
  {#each codes as code (code)}<li>{code}</li>{/each}
</ul>
<div class="mt-3 flex flex-wrap gap-3">
  <button class={button} type="button" onclick={copy}>{copied ? 'Copied' : 'Copy'}</button>
  <button class={button} type="button" onclick={download}>Download</button>
</div>
<p class="mt-2 text-sm font-semibold text-bad" role="alert">{error}</p>
<label class="mt-4 flex items-center gap-2 text-sm">
  <input class="accent-accent" type="checkbox" bind:checked={saved} /> I saved my recovery codes
</label>
<button class="{primary} mt-4 w-full" type="button" disabled={!saved} onclick={onDone}>Continue</button>
