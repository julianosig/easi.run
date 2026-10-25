<script lang="ts">
  // The screens before a login (create the password, the second step it offers, log in), as the landing opens: no card,
  // the whole window. On one side the landing's ground (white fading through the wash to the soft blue it ends in, its
  // blue glow), the mark, its headline and a browser where your agent and you share the page; on the other the form,
  // under a label with your coral dot that says where the person is, and the line about the source. On a narrow screen
  // the landing's side comes first, shorter. The page around it is padded for the views after a login: this one spans
  // the window over that padding (see App).
  import type { Snippet } from 'svelte';
  import BrowserScene from './BrowserScene.svelte';
  import { demoOf } from './demos.ts';
  import Icon from './Icon.svelte';
  import SourceNote from './SourceNote.svelte';
  import Wordmark from './Wordmark.svelte';

  let { label, children }: { label: string; children: Snippet } = $props();
  /** What a session does, as the landing's demos show it. */
  const FEATURES = ['takeover', 'drag', 'download', 'sound', 'clipboard'].map(demoOf);
</script>

<div class="-mx-4 -mb-12 -mt-3 grid min-h-dvh bg-paper lg:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)]">
  <div class="relative isolate flex flex-col overflow-hidden bg-linear-to-b from-paper via-wash to-paper px-6 pt-6 sm:px-10 lg:sticky lg:top-0 lg:h-dvh lg:to-accent-soft lg:px-[clamp(2.5rem,4.5vw,5rem)] lg:pt-9">
    <div class="pointer-events-none absolute inset-0 -z-10" aria-hidden="true">
      <div class="absolute -left-[20%] top-[30%] h-[46rem] w-[60rem] bg-[radial-gradient(closest-side,rgb(36_87_245/0.16),rgb(120_160_255/0.07)_55%,transparent)]"></div>
      <div class="dots absolute inset-0"></div>
    </div>
    <Wordmark class="text-[22px]" />
    <p class="mt-[clamp(1.5rem,7vh,5.5rem)] max-w-[36rem] text-[clamp(1.75rem,3.5vw,3.5rem)] font-semibold leading-[1.06] tracking-[-0.035em] text-balance">
      Isolated browser sessions for your agents. <span class="text-accent">Self-hosted. Yours.</span>
    </p>
    <ul class="mt-6 hidden max-w-[36rem] flex-wrap gap-x-5 gap-y-2.5 sm:flex" aria-label="In every session">
      {#each FEATURES as feature (feature.title)}
        <li class="flex items-center gap-2 text-sm font-semibold text-ink/70"><Icon d={feature.icon} class="size-4 text-accent" />{feature.title}</li>
      {/each}
    </ul>
    <BrowserScene class="mt-7 -mr-6 h-[14rem] mask-b-from-70% sm:-mr-10 sm:h-[17rem] lg:mask-none lg:mt-auto lg:-mr-[clamp(2.5rem,4.5vw,5rem)] lg:h-[min(52vh,30rem)]" />
  </div>

  <main class="flex flex-col px-6 sm:px-10 lg:px-[clamp(2.5rem,5vw,6rem)]">
    <div class="flex flex-1 items-center py-10 lg:py-16">
      <div class="mx-auto w-full max-w-[27rem]">
        <p class="mb-4 inline-flex items-center gap-2 text-sm font-semibold uppercase tracking-[0.14em] text-accent">
          <span class="size-2 rounded-full bg-you" aria-hidden="true"></span>{label}
        </p>
        {@render children()}
      </div>
    </div>
    <SourceNote class="pb-7 pt-4" />
  </main>
</div>

<style>
  /* The landing's ground, with a fine grid of dots that fades out towards the browser. */
  .dots {
    background-image: radial-gradient(rgb(20 40 75 / 0.13) 1px, transparent 1.2px);
    background-size: 22px 22px;
    mask-image: radial-gradient(ellipse 80% 65% at 30% 25%, #000 20%, transparent 75%);
  }
</style>
