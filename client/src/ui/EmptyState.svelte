<script lang="ts">
  // A page with nothing on it yet: its icon, what is missing, one plain line, and the button that makes the first.
  import Icon from './Icon.svelte';
  import NewButton from './NewButton.svelte';

  let { icon, title, text, label, onclick }: { icon: string; title: string; text: string; label: string; onclick: () => void } =
    $props();
</script>

<div class="mt-20 text-center">
  <span class="mx-auto mb-4 grid size-14 place-items-center rounded-2xl bg-accent-soft text-accent"><Icon d={icon} class="size-7" /></span>
  <p class="text-lg font-semibold">{title}</p>
  <p class="mx-auto mt-1.5 max-w-sm text-sm text-muted">{text}</p>
  <NewButton {label} {onclick} class="mt-6" />
</div>
