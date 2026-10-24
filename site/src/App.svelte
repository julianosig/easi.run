<script lang="ts">
  import Compare from './lib/Compare.svelte';
  import { TECH } from './lib/tech.ts';
  import Nav from './lib/Nav.svelte';
  import SectionHeader from './lib/SectionHeader.svelte';
  import { REPO } from './lib/site.ts';
  import LiveDemo from './lib/LiveDemo.svelte';
  import Wander from './lib/Wander.svelte';
  import { followVisibility } from './lib/motion.ts';
  import { focus } from './lib/focus.ts';

  $effect(followVisibility);

  const doc = (path: string) => `${REPO}/blob/main/docs/${path}`;

  const wrap = 'mx-auto max-w-[70rem] px-6';
</script>

<a href="#main" class="sr-only focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-50 focus:rounded-lg focus:bg-surface focus:px-3 focus:py-2 focus:shadow-lg"
  >Skip to content</a
>

<Wander />
<Nav />

<main id="main">
  <!-- Hero -->
  <section id="demo" class="relative overflow-x-clip">
    <div class="pointer-events-none absolute left-1/2 top-[26rem] -z-10 h-[44rem] w-[90rem] -translate-x-1/2 bg-[radial-gradient(closest-side,rgb(36_87_245/0.14),rgb(120_160_255/0.07)_55%,transparent)]" aria-hidden="true"></div>
    <div class="{wrap} pt-20 text-center">
      <h1 class="mx-auto max-w-3xl text-[clamp(2rem,3.6vw,3.25rem)] font-semibold leading-[1.08] tracking-[-0.03em]">
        Isolated browser sessions for your agents. <span class="text-accent">Self-hosted. Yours.</span>
      </h1>
    </div>

    <div class={wrap}>
      <LiveDemo />
    </div>

    <!-- What I am built on, under the preview. -->
    <div class="{wrap} mt-14 text-center">
      <ul class="flex flex-wrap items-center justify-center gap-x-6 gap-y-3" aria-label="Built on">
        {#each TECH as tech (tech.name)}
          <li class="flex items-center gap-2 text-sm text-muted">
            <svg viewBox="0 0 24 24" class="size-5 fill-current" aria-hidden="true"><path d={tech.d} /></svg>{tech.name}
          </li>
        {/each}
      </ul>
    </div>
  </section>

  <!-- Compare -->
  <section id="compare" class="{wrap} py-24">
    <SectionHeader label="Compare">Anyone vs. me.</SectionHeader>
    <p class="mx-auto mt-4 max-w-4xl text-center text-base leading-relaxed text-muted">
      What hurts when your agents run on someone else’s browsers: hourly bills, logins only a person can finish,
      passwords on a vendor’s machine, files and sound that never reach you. Here is who solves it.
    </p>
    <div class="mt-10"><Compare /></div>
    <p class="mt-5 text-center text-xs text-muted">
      ✓ has it · – doesn’t, partly, unknown, or not applicable · Tap a mark for details · Oct 2026 ·
      <a href={doc('comparison.md')} class="font-medium text-accent hover:underline {focus}">Sources</a> · Names and logos
      belong to their owners.
    </p>
  </section>

</main>

<!-- The last word: the name written large, in its own colors. Decoration only, so no footer landmark: it would be
     empty to assistive technology. -->
<div class="relative overflow-hidden bg-linear-to-b from-paper via-wash to-accent-soft">
  <p
    class="pointer-events-none select-none whitespace-nowrap pb-[2vw] pt-20 text-center font-display text-[clamp(5rem,23vw,24rem)] font-bold leading-[0.8] tracking-[-0.06em] text-ink"
    aria-hidden="true"
  >
    easi<span class="text-you">.</span>run
  </p>
</div>
