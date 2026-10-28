<script lang="ts">
  // A page with nothing on it yet: what the feature is for, in one line, and two short points on what it gives, on a
  // plain background where the page's content starts. The page's own button (in its header, where it stays once there
  // is something) makes the first.
  import { CHECK_ICON } from './admin.ts';
  import Icon from './Icon.svelte';

  let { icon, title, text, points }: { icon: string; title: string; text: string; points: [string, string] } = $props();
</script>

<section class="mt-8 max-w-3xl rounded-2xl bg-wash p-5">
  <div class="flex items-start gap-4">
    <span class="grid size-10 shrink-0 place-items-center rounded-xl bg-accent-soft text-accent"><Icon d={icon} class="size-5" /></span>
    <div class="min-w-0">
      <h2 class="text-base font-semibold">{title}</h2>
      <p class="mt-0.5 text-sm text-muted">{text}</p>
    </div>
  </div>
  <ul class="mt-4 space-y-2 pl-14">
    {#each points as point (point)}
      <li class="flex items-start gap-2.5 text-sm">
        <Icon d={CHECK_ICON} class="mt-0.5 size-4 shrink-0 text-accent" />
        <span>{point}</span>
      </li>
    {/each}
  </ul>
</section>
