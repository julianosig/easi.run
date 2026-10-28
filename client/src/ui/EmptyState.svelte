<script lang="ts">
  // A page with nothing on it yet: what the feature is for, in one line, and two short points on what it gives. The
  // page's own button (in its header, where it stays once there is something) makes the first.
  import { CHECK_ICON } from './admin.ts';
  import Icon from './Icon.svelte';

  let { icon, title, text, points }: { icon: string; title: string; text: string; points: [string, string] } = $props();
</script>

<section class="mx-auto mt-14 max-w-xl rounded-2xl border border-line bg-paper p-7 sm:p-8">
  <div class="flex items-start gap-4">
    <span class="grid size-12 shrink-0 place-items-center rounded-2xl bg-accent-soft text-accent"><Icon d={icon} class="size-6" /></span>
    <div class="min-w-0">
      <h2 class="text-lg font-semibold">{title}</h2>
      <p class="mt-1 text-sm text-muted">{text}</p>
    </div>
  </div>
  <ul class="mt-5 space-y-2.5 border-t border-line pt-5">
    {#each points as point (point)}
      <li class="flex items-start gap-2.5 text-sm">
        <Icon d={CHECK_ICON} class="mt-0.5 size-4 shrink-0 text-accent" />
        <span>{point}</span>
      </li>
    {/each}
  </ul>
</section>
