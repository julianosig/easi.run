<script lang="ts">
  // The page's look: the system's, or one the person picks. For the landing and the docs alike, each giving its words.
  import { chooseTheme, chosenTheme, THEMES, type Theme } from '../../../i18n/theme.ts';
  import { focus } from './focus.ts';

  let { label, names }: { label: string; names: Record<Theme, string> } = $props();
  let theme: Theme = $state('system');
  // After the first paint of a page rendered ahead of time, which knows no one's pick.
  $effect(() => {
    theme = chosenTheme();
  });
</script>

<label class="relative flex items-center rounded-xl px-2 py-2 text-muted transition hover:text-ink" title={label}>
  <svg viewBox="0 0 24 24" class="size-[18px] text-accent/70" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"
    ><path d="M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 3v18M12 7h6M12 11h8.5M12 15h7" /></svg
  >
  <span class="sr-only">{label}</span>
  <select class="absolute inset-0 cursor-pointer appearance-none opacity-0 {focus}" bind:value={theme} onchange={() => chooseTheme(theme)}>
    {#each THEMES as option (option)}<option value={option}>{names[option]}</option>{/each}
  </select>
</label>
