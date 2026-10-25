<script lang="ts">
  // An option that is on or off: its icon, its name and one plain line, and a switch (a checkbox announced as one).
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  let {
    icon,
    title,
    help = '',
    checked = $bindable(false),
    disabled = false,
    onchange,
    children,
  }: {
    icon: string;
    title: string;
    help?: string;
    checked?: boolean;
    disabled?: boolean;
    /** Called with the new state, after a change by the person. */
    onchange?: (on: boolean) => void;
    /** Shown under the line (why it cannot be turned on, say). */
    children?: Snippet;
  } = $props();
  const id = `switch-${Math.random().toString(36).slice(2, 10)}`;
</script>

<div class="flex items-start gap-3 rounded-xl px-3 py-3 transition hover:bg-wash focus-within:bg-wash">
  <span class="mt-0.5 grid size-9 shrink-0 place-items-center rounded-xl bg-accent-soft text-accent"><Icon d={icon} class="shrink-0 size-[18px]" /></span>
  <span class="min-w-0 flex-1">
    <label class="block text-[15px] font-semibold" for={id}>{title}</label>
    {#if help}<span id="{id}-help" class="block text-[13px] leading-snug text-muted">{help}</span>{/if}
    {@render children?.()}
  </span>
  <span class="relative mt-1.5 inline-flex shrink-0">
    <input
      {id}
      class="peer absolute inset-0 z-10 m-0 cursor-pointer opacity-0 disabled:cursor-default"
      type="checkbox"
      role="switch"
      aria-describedby={help ? `${id}-help` : undefined}
      bind:checked
      {disabled}
      onchange={(e) => onchange?.(e.currentTarget.checked)}
    />
    <span
      class="h-6 w-11 rounded-full bg-line transition peer-checked:bg-accent peer-disabled:opacity-50 peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-accent after:absolute after:left-0.5 after:top-0.5 after:size-5 after:rounded-full after:bg-paper after:shadow-sm after:transition peer-checked:after:translate-x-5"
      aria-hidden="true"
    ></span>
  </span>
</div>
