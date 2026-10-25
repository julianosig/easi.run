<script lang="ts">
  import { api, messageOf, setCsrf, wrongOrigin } from './api.ts';
  import { card, field, label, primary } from './ui.ts';
  import WrongAddress from './WrongAddress.svelte';

  let { onDone }: { onDone: () => void } = $props();
  let password = $state('');
  let error = $state('');
  /** The address the admin is set up for, when it refused this one. */
  let elsewhere: string | null = $state(null);
  let input: HTMLInputElement | undefined = $state();
  $effect(() => input?.focus());

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    try {
      setCsrf((await api<{ csrf: string }>('POST', '/api/login', { password })).csrf);
      onDone();
    } catch (e) {
      elsewhere = wrongOrigin(e);
      error = messageOf(e);
    }
  }
</script>

<form class="{card} mx-auto mt-16 max-w-md" onsubmit={submit}>
  <h1 class="text-2xl font-semibold tracking-[-0.02em]">Log in</h1>
  <label class={label} for="password">Password</label>
  <input id="password" class={field} type="password" autocomplete="current-password" required bind:value={password} bind:this={input} />
  <button class="{primary} mt-6 w-full" type="submit">Log in</button>
  {#if elsewhere}
    <WrongAddress origin={elsewhere} href={`${elsewhere}${location.pathname}`} />
  {:else}
    <p class="mt-3 font-semibold text-bad" role="alert">{error}</p>
  {/if}
</form>
