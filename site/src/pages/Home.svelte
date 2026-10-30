<script lang="ts">
  // The first page: the promise, the live demo to try it, and what I am built on.
  import { TECH } from '../lib/tech.ts';
  import LiveDemo from '../lib/LiveDemo.svelte';
  import { t } from '../i18n/index.ts';

  const wrap = 'mx-auto max-w-[70rem] px-6';
</script>

<section id="demo" class="relative overflow-x-clip pb-12">
  <div class="pointer-events-none absolute left-1/2 top-[26rem] -z-10 h-[44rem] w-[90rem] -translate-x-1/2 bg-[radial-gradient(closest-side,rgb(36_87_245/0.14),rgb(120_160_255/0.07)_55%,transparent)]" aria-hidden="true"></div>
  <div class="{wrap} pt-20 text-center">
    <h1 class="mx-auto max-w-4xl text-[clamp(2rem,3.6vw,3.25rem)] font-semibold leading-[1.08] tracking-[-0.03em]">
      {t('page.hero.title')} <span class="block text-accent">{t('page.hero.accent')}</span>
    </h1>
    <p class="mx-auto mt-5 max-w-5xl text-pretty text-lg leading-relaxed text-muted">{t('page.hero.lead')}</p>
  </div>

  <div class={wrap}>
    <LiveDemo />
  </div>

  <!-- What I am built on, under the preview. -->
  <div class="{wrap} mt-14 text-center">
    <ul class="flex flex-wrap items-center justify-center gap-x-6 gap-y-3" aria-label={t('page.builtOn')}>
      {#each TECH as tech (tech.name)}
        <li class="flex items-center gap-2 text-sm text-muted">
          <svg viewBox="0 0 24 24" class="size-5 fill-current" aria-hidden="true"><path d={tech.d} /></svg>{tech.name}
        </li>
      {/each}
    </ul>
  </div>
</section>
