<script lang="ts">
  // A page with nothing on it yet: short points on what the feature gives, checked, on a plain background where the
  // page's content starts. A point is a text, or a snippet when it holds more than text (an address in code). The
  // page's add (in its header, where it stays once there is something) makes the first.
  import type { Snippet } from 'svelte';
  import { ICONS } from './icons.ts';
  import Icon from './Icon.svelte';

  let { points }: { points: (string | Snippet)[] } = $props();
</script>

<ul class="mt-8 space-y-2.5 rounded-2xl bg-wash p-5">
  {#each points as point, i (i)}
    <li class="flex items-start gap-2.5 text-sm">
      <Icon d={ICONS.check} class="mt-0.5 size-4 shrink-0 text-accent" />
      <span>{#if typeof point === 'string'}{point}{:else}{@render point()}{/if}</span>
    </li>
  {/each}
</ul>
