<script lang="ts">
  // The step after the demo: the one prompt that installs it, to copy into a coding agent on a server, and the quick
  // start for doing it by hand. The prompt is the docs' own (src/docs/pages/QuickStart.svelte), in English for the agent.
  import { NAME, ORIGIN } from './site.ts';
  import { focus } from './focus.ts';
  import { locale, pathOf, t } from '../i18n/index.ts';

  const prompt = `Read ${ORIGIN}/install.md and install ${NAME} on this server.`;
  let copied = $state(false);
  let reset: ReturnType<typeof setTimeout> | undefined;

  async function copy() {
    try {
      await navigator.clipboard.writeText(prompt);
      copied = true;
      clearTimeout(reset);
      reset = setTimeout(() => (copied = false), 2000);
    } catch {
      // Selecting the text by hand still works.
    }
  }
</script>

<div class="mx-auto mt-10 max-w-3xl rounded-2xl border border-line bg-surface p-5 text-left shadow-[0_10px_34px_-18px_rgb(20_40_75/0.25)] sm:p-6">
  <h2 class="text-lg font-semibold tracking-[-0.01em]">{t('features.install.title')}</h2>
  <p class="mt-1 text-sm leading-relaxed text-muted">{t('page.install.lead')}</p>
  <div class="mt-4 flex items-center gap-2 rounded-xl bg-void py-1.5 pl-4 pr-1.5">
    <code class="min-w-0 flex-1 overflow-x-auto whitespace-nowrap py-1.5 font-mono max-sm:whitespace-normal max-sm:[overflow-wrap:anywhere] text-[13.5px] text-white/90">{prompt}</code>
    <button
      class="shrink-0 rounded-lg bg-white/10 px-3 py-1.5 text-sm font-semibold text-white transition hover:bg-white/20 {focus}"
      type="button"
      onclick={copy}
      aria-live="polite">{copied ? t('page.install.copied') : t('page.install.copy')}</button
    >
  </div>
  <p class="mt-3 text-sm text-muted">
    {t('page.install.manual')}
    <a href={`https://docs.easi.run${pathOf(locale())}#/quick-start`} class="font-medium text-accent hover:underline {focus}">{t('page.install.guide')}</a>
  </p>
</div>
