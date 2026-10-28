<script lang="ts">
  // The admin's one button: what it is for (variant), its size, and an icon before its words, or alone; then `label`
  // names it, for screen readers and as its tooltip.
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import Icon from './Icon.svelte';
  import { buttonClass, type ButtonSize, type ButtonTone, type ButtonVariant } from './ui.ts';

  let {
    variant = 'secondary',
    size = 'md',
    tone = 'neutral',
    icon,
    label,
    children,
    ref = $bindable(),
    class: extra = '',
    ...rest
  }: {
    variant?: ButtonVariant;
    size?: ButtonSize;
    tone?: ButtonTone;
    icon?: string | undefined;
    /** An icon alone's name. */
    label?: string | undefined;
    children?: Snippet;
    ref?: HTMLButtonElement | undefined;
    class?: string;
  } & Omit<HTMLButtonAttributes, 'class' | 'children'> = $props();
</script>

<button
  bind:this={ref}
  type="button"
  class="{buttonClass(variant, size, !children, tone)} {extra}"
  aria-label={children ? undefined : label}
  title={children ? undefined : label}
  {...rest}
>
  {#if icon}<Icon d={icon} class={size === 'sm' ? 'size-4' : size === 'xl' ? 'size-7' : 'size-[18px]'} />{/if}{@render children?.()}
</button>
