<script lang="ts">
  // The current password, asked again before a change to how one signs in (see `Security`): the change runs with it,
  // and what went wrong, if anything, shows here.
  import { button, field, label, primary } from './ui.ts';

  let {
    title,
    action,
    onConfirm,
    onCancel,
  }: {
    title: string;
    /** The confirming button's label: what the change does. */
    action: string;
    /** Makes the change; why it failed, or null. */
    onConfirm: (password: string) => Promise<string | null>;
    onCancel: () => void;
  } = $props();
  let password = $state('');
  let error = $state('');
  let busy = $state(false);
  let input: HTMLInputElement | undefined = $state();
  $effect(() => input?.focus());

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = (await onConfirm(password)) ?? '';
    busy = false;
  }
</script>

<form class="mt-4 rounded-xl border border-line bg-wash p-4" onsubmit={submit}>
  <p class="font-semibold">{title}</p>
  <label class={label} for="current-password">Your current password</label>
  <input id="current-password" class={field} type="password" autocomplete="current-password" required bind:value={password} bind:this={input} />
  <div class="mt-4 flex gap-3">
    <button class={primary} type="submit" disabled={busy}>{action}</button>
    <button class={button} type="button" onclick={onCancel}>Cancel</button>
  </div>
  <p class="mt-3 font-semibold text-bad" role="alert">{error}</p>
</form>
