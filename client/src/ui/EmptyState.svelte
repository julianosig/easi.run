<script lang="ts">
  // A page with nothing on it yet: two short points on what the feature gives, checked, on a plain background where the
  // page's content starts. The page's add (in its header, where it stays once there is something) makes the first.
  import { ICONS } from './icons.ts';
  import Icon from './Icon.svelte';

  let { points }: { points: [string, string] } = $props();
</script>

<ul class="mt-8 space-y-2.5 rounded-2xl bg-wash p-5">
  {#each points as point (point)}
    <li class="flex items-start gap-2.5 text-sm">
      <Icon d={ICONS.check} class="mt-0.5 size-4 shrink-0 text-accent" />
      <span>{point}</span>
    </li>
  {/each}
</ul>
