<script lang="ts">
  // A command to run, as the landing's code blocks show one (site/src/docs/Code.svelte): where it runs, the shell's
  // prompt in your coral, and a button that copies the command alone. A long command wraps at its spaces only, never
  // inside a word (nor at a word's hyphens), its next lines under the command's start.
  import { t } from '../i18n/index.svelte.ts';
  import { copyText } from './api.ts';
  import Icon from './Icon.svelte';

  let { command, where, name }: { command: string; where: string; /** What the copy button copies, for its label. */ name: string } = $props();
  let copied = $state(false);
  let refused = $state(false);
  let text: HTMLElement | undefined = $state();
  let reset: ReturnType<typeof setTimeout> | undefined;
  /** The latest copy asked for: an earlier one the clipboard answers later changes nothing. */
  let attempt = 0;
  let live = true;
  $effect(() => () => {
    live = false;
    clearTimeout(reset);
  });

  async function copy() {
    const mine = ++attempt;
    const ok = await copyText(command);
    if (!live || mine !== attempt) return;
    clearTimeout(reset);
    if (ok) {
      refused = false;
      copied = true;
      // Each copy shows "Copied" for the full two seconds, however soon after the last one it comes.
      reset = setTimeout(() => (copied = false), 2000);
    } else {
      // The browser refused the clipboard: the command is selected, for the person to copy.
      copied = false;
      refused = true;
      if (text) getSelection()?.selectAllChildren(text);
    }
  }
</script>

<div class="mt-2.5 overflow-hidden rounded-xl bg-void text-[13.5px] leading-6 text-white/90 ring-1 ring-ink/10">
  <div class="flex items-center justify-between gap-3 border-b border-white/10 py-1 pl-4 pr-1.5 text-xs font-semibold text-white/50">
    <span>{where}</span>
    <button
      class="inline-flex items-center gap-1.5 rounded-md px-2 py-1.5 transition hover:bg-white/10 hover:text-white focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-[#8fb0ff] {copied ? 'text-[#9fe3b8]' : 'text-white/70'}"
      type="button"
      aria-label={copied ? t('agents.command.copied', { name }) : t('agents.command.copy', { name })}
      onclick={copy}
    >
      <Icon d={copied ? 'M5 12.5l4.5 4.5L19 7.5' : 'M9 9h10v11H9zM5 15V4h10'} class="size-3.5" stroke={2} />{copied ? t('agents.copied') : t('agents.copy')}
    </button>
  </div>
  <pre class="whitespace-pre-wrap py-3 pl-[calc(1rem+2ch)] pr-4 -indent-[2ch] font-mono [overflow-wrap:normal]"><span class="select-none text-you" aria-hidden="true">$ </span><code bind:this={text}
      >{#each command.split(' ') as word, i (i)}{i ? ' ' : ''}<span class="whitespace-nowrap">{word}</span>{/each}</code
    ></pre>
  <p class={refused && !copied ? 'border-t border-white/10 px-4 py-2 text-xs text-white/70' : 'sr-only'} role="status">{copied ? t('agents.copied') : refused ? t('agents.command.refused') : ''}</p>
</div>
