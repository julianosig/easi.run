<script lang="ts">
  import { api, messageOf } from './api.ts';
  import { card, field, label, primary } from './ui.ts';

  let { onDone }: { onDone: () => void } = $props();
  let token = $state('');
  let password = $state('');
  let error = $state('');

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    try {
      await api('POST', '/api/enroll', { token: token.trim(), password });
      onDone();
    } catch (e) {
      error = messageOf(e);
    }
  }
</script>

<form class="{card} mx-auto mt-16 max-w-md" onsubmit={submit}>
  <h1 class="text-2xl font-semibold tracking-[-0.02em]">Set the password</h1>
  <p class="mt-1 text-muted">Use the one-time enrollment token printed in the admin application’s log.</p>
  <label class={label} for="token">Enrollment token</label>
  <input id="token" class={field} type="text" autocomplete="off" required bind:value={token} />
  <label class={label} for="password">Password (12 characters or more)</label>
  <input id="password" class={field} type="password" autocomplete="new-password" minlength="12" required bind:value={password} />
  <button class="{primary} mt-6 w-full" type="submit">Set password</button>
  <p class="mt-3 font-semibold text-bad" role="alert">{error}</p>
</form>
