<script lang="ts">
  // Head to head: this product against one rival. The rivals take turns every two seconds until the visitor's first
  // click anywhere in the comparison; clicking the rival's name opens every rival in a list. Our column is the
  // highlighted one; the rival's facts stay just as readable.
  import Wordmark from './Wordmark.svelte';
  import { focus } from './focus.ts';
  import { reducedMotion } from './motion.ts';
  import { GROUPS, OURS, RIVALS, ROWS, type Mark } from './content.ts';
  import { NAME } from './site.ts';
  import { say, t } from '../i18n/index.ts';

  let picked = $state(RIVALS[0].product);
  const rival = $derived(RIVALS.find((r) => r.product === picked) ?? RIVALS[0]);

  const MARKS: Record<Mark, string> = {
    yes: t('compare.mark.yes'),
    partial: t('compare.mark.partial'),
    no: t('compare.mark.no'),
    unknown: t('compare.mark.unknown'),
    na: t('compare.mark.na'),
  };
  const ROTATE_MS = 2000;

  let open = $state(false);
  // Rivals take turns until the visitor first acts on the comparison, and never for one who asked for less motion.
  let rotating = $state(true);
  let root: HTMLDivElement | undefined = $state();
  // The turns pause while nobody can see them: the comparison off screen, or the tab hidden.
  let seen = $state(false);

  $effect(() => {
    if (!root) return;
    let onScreen = false;
    const sync = () => (seen = onScreen && !document.hidden);
    const observer = new IntersectionObserver(([entry]) => {
      onScreen = entry.isIntersecting;
      sync();
    });
    observer.observe(root);
    document.addEventListener('visibilitychange', sync);
    return () => {
      observer.disconnect();
      document.removeEventListener('visibilitychange', sync);
    };
  });
  $effect(() => {
    if (!rotating || !seen || reducedMotion.current) return;
    const timer = setInterval(() => {
      const next = (RIVALS.findIndex((r) => r.product === picked) + 1) % RIVALS.length;
      picked = RIVALS[next].product;
    }, ROTATE_MS);
    return () => clearInterval(timer);
  });
  let shown = $state('');
  let trigger: HTMLButtonElement | undefined = $state();
  let panel: HTMLDivElement | undefined = $state();

  // Choosing from the list closes it and returns focus to its button; Escape does the same; a click elsewhere closes
  // it and leaves focus where the visitor clicked.
  function choose(product: string) {
    picked = product;
    open = false;
    trigger?.focus();
  }
  function onKey(event: KeyboardEvent) {
    if (shown && event.key === 'Escape') shown = '';
    if (open && event.key === 'Escape') {
      open = false;
      trigger?.focus();
    }
  }
  function onPointer(event: PointerEvent) {
    if (root?.contains(event.target as Node)) rotating = false;
    if (shown && !(event.target as Element).closest?.('[aria-expanded="true"]')) shown = '';
    if (open && !panel?.contains(event.target as Node) && !trigger?.contains(event.target as Node)) open = false;
  }
</script>

<!-- A cell is checked or not. What sits behind it (plan, condition, source wording) opens on click or tap, closes on a
     second click, Escape or a click elsewhere, and is part of the button's name for screen readers. -->
{#snippet verdict(mark: Mark, text: string, ours: boolean, key: string)}
  {@const ok = mark === 'yes'}
  <button
    type="button"
    class="relative inline-grid size-7 place-items-center rounded-full {focus}"
    aria-expanded={shown === key}
    onclick={() => (shown = shown === key ? '' : key)}
  >
    <span class="grid size-[22px] place-items-center rounded-full {ok ? (ours ? 'bg-accent text-white' : 'bg-muted text-white') : 'text-ink/55 hover:bg-wash'}">
      <svg viewBox="0 0 20 20" class="size-[11px]" fill="none" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        {#if ok}<path d="M5 10.5l3.2 3L15 6.5" />{:else}<path d="M6 10h8" />{/if}
      </svg>
    </span>
    <span class="sr-only">{MARKS[mark]}: {text}</span>
    {#if shown === key}
      <span class="absolute bottom-full left-1/2 z-20 mb-2 w-max max-w-64 -translate-x-1/2 rounded-lg bg-ink px-3 py-2 text-left text-xs leading-snug text-white shadow-lg" aria-hidden="true"
        >{text}</span
      >
    {/if}
  </button>
{/snippet}

<svelte:window onkeydown={onKey} onpointerdown={onPointer} />

<div class="relative" bind:this={root} onfocusin={() => (rotating = false)}>
    <div
      bind:this={panel}
      id="rivals"
      class="absolute right-0 top-24 z-30 w-[min(46rem,100%)] rounded-2xl border border-line bg-surface p-5 text-left shadow-[0_24px_60px_-20px_rgb(23_26_36/0.35)]"
      hidden={!open}
    >
      {#each GROUPS as group (group)}
        <p class="mb-2 mt-4 text-xs font-medium text-muted first:mt-0">{say(`compare.group.${group}`)}</p>
        <ul class="grid grid-cols-3 gap-1">
          {#each RIVALS.filter((r) => r.group === group) as r (r.product)}
            {@const on = r.product === picked}
            <li>
              <button
                type="button"
                class="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-sm transition {focus} {on
                  ? 'bg-accent-soft font-medium text-ink'
                  : 'text-ink/80 hover:bg-paper'}"
                aria-pressed={on}
                onclick={() => choose(r.product)}
              >
                <img src={`/logos/${r.logo}`} alt="" loading="lazy" class="size-5 shrink-0 rounded bg-white object-contain p-px ring-1 ring-line" />
                <span class="truncate">{r.product}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/each}
    </div>

  <!-- Positioned, so the cells' screen-reader labels (absolute) scroll with the table instead of widening the page. -->
  <div class="relative overflow-x-auto rounded-2xl border border-line bg-surface p-2 shadow-[0_24px_60px_-36px_rgb(23_26_36/0.35)]">
  <table class="w-full min-w-[40rem] table-fixed border-separate border-spacing-0 text-[15px]">
    <caption class="sr-only">{t('compare.caption', { name: NAME, rival: rival.product })}</caption>
    <colgroup><col /><col class="w-[15rem]" /><col class="w-[15rem]" /></colgroup>
    <thead>
      <tr>
        <th scope="col"><span class="sr-only">{t('compare.feature')}</span></th>
        <th scope="col" class="rounded-t-2xl bg-accent px-5 py-4 text-white">
          <Wordmark class="text-2xl" mark="inverse" />
        </th>
        <th scope="col" class="border-b border-line px-5 py-4">
          <button
            bind:this={trigger}
            type="button"
            class="mx-auto flex items-center gap-2 rounded-xl px-2 py-1 transition hover:bg-wash {focus}"
            aria-expanded={open}
            aria-controls="rivals"
            onclick={() => (open = !open)}
          >
            {#key rival.product}
              <span class="appear flex h-14 items-center gap-3 text-lg font-semibold tracking-tight text-ink">
                <img src={`/logos/${rival.logo}`} alt="" loading="lazy" class="size-8 shrink-0 rounded-lg bg-white object-contain p-1 ring-1 ring-line" />
                <!-- Two lines fit the longest name, so a turn never moves the rows below. -->
                <span class="line-clamp-2 text-left leading-tight">{rival.product}</span>
              </span>
            {/key}
            <span class="sr-only">{t('compare.choose', { count: RIVALS.length })}</span>
            <svg viewBox="0 0 12 12" class="size-3 text-muted transition {open ? 'rotate-180' : ''}" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="M3 4.5l3 3 3-3" /></svg>
          </button>
        </th>
      </tr>
    </thead>
    <tbody>
      {#each ROWS as row, i (row)}
        {@const last = i === ROWS.length - 1}
        <tr>
          <th scope="row" class="py-2 pl-5 pr-4 text-left align-middle text-sm font-medium text-ink {last ? '' : 'border-b border-line'}">{say(`compare.row.${row}`)}</th>
          <td class="border-x-2 border-accent bg-accent-soft px-5 py-1 text-center align-middle {last ? 'rounded-b-2xl border-b-2' : 'border-b border-b-accent/15'}">
            {@render verdict(OURS[row], say(`ours.${row}`), true, `ours-${row}`)}
          </td>
          <td class="px-5 py-1 text-center align-middle {last ? '' : 'border-b border-line'}">
            {#key rival.product}<div class="appear">{@render verdict(rival.cells[row], say(`rivals.${rival.id}.${row}`), false, `rival-${row}`)}</div>{/key}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
  </div>
</div>

<style>
  .appear {
    animation: appear 0.35s ease-out both;
  }
  @keyframes appear {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
</style>
