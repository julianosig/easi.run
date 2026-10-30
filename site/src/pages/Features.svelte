<script lang="ts">
  // The features' page: the headline and its lead, the two cards the landing first showed side by side, then every
  // other feature in its group, each a card with its own drawing.
  import SectionHeader from '../lib/SectionHeader.svelte';
  import { type Card, FIRST, GROUPS } from '../lib/features/index.ts';
  import { playing } from '../lib/features/playing.ts';
  import { say, t } from '../i18n/index.ts';
</script>

{#snippet card({ id, art: Art }: Card, heading: 'h2' | 'h3')}
  <li class="rounded-2xl bg-wash p-6" {@attach playing}>
    <Art />
    <svelte:element this={heading} class="mt-5 text-lg font-semibold tracking-tight">{say(`features.${id}.title`)}</svelte:element>
    <p class="mt-2 text-[15px] leading-relaxed text-muted">{say(`features.${id}.text`)}</p>
  </li>
{/snippet}

<div class="mx-auto max-w-[70rem] px-6 pb-24 pt-20">
  <SectionHeader tag="h1">{t('features.title')}</SectionHeader>
  <p class="mx-auto mt-4 max-w-4xl text-center text-base leading-relaxed text-muted">{t('features.intro')}</p>

  <ul class="mx-auto mt-12 grid max-w-4xl gap-4 sm:grid-cols-2">
    {#each FIRST as first (first.id)}{@render card(first, 'h2')}{/each}
  </ul>

  {#each GROUPS as group (group.id)}
    <section class="mt-20" aria-labelledby="features-{group.id}">
      <h2 id="features-{group.id}" class="text-center text-[clamp(1.5rem,2.4vw,2rem)] font-semibold tracking-[-0.02em]">{say(`features.group.${group.id}`)}</h2>
      <ul class="mt-8 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {#each group.cards as each (each.id)}{@render card(each, 'h3')}{/each}
      </ul>
    </section>
  {/each}
</div>
