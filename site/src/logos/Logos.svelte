<script lang="ts">
  // Logo proposals (unlisted, /logos/), round four: six marks built on the mouse pointer, the agent's and yours,
  // all in the current palette (brand blue, coral for you, navy ink, light blue washes), on white, wash and navy,
  // with the mark at favicon sizes, beside the current logo for reference. The landing is untouched.
  import Wordmark from '../lib/Wordmark.svelte';

  const palette = [
    { name: 'Brand blue', hex: '#2457F5' },
    { name: 'Deep blue', hex: '#1A45D6' },
    { name: 'Coral (you)', hex: '#F06445' },
    { name: 'Navy ink', hex: '#14284B' },
    { name: 'Soft blue', hex: '#EAF0FF' },
  ];

  type Ground = 'paper' | 'wash' | 'navy';
  const grounds: Ground[] = ['paper', 'wash', 'navy'];
  const ground: Record<Ground, string> = { paper: 'bg-paper border border-line', wash: 'bg-wash', navy: 'bg-ink' };

  const options = [
    { key: 'duo', name: 'Two pointers', idea: 'The agent’s blue pointer in front, yours in coral right behind it: one screen, shared, and you can take over anytime.' },
    { key: 'click', name: 'One click', idea: 'The classic pointer with a coral click at its tip: one click and you are in control.' },
    { key: 'dash', name: 'Pointer at speed', idea: 'The pointer leaning forward with speed lines behind it: the agent works fast, and you see it live.' },
    { key: 'tile', name: 'Pointer tile', idea: 'A white pointer on a blue tile with a coral light: an app icon first, made for favicons and docks.' },
    { key: 'split', name: 'Shared pointer', idea: 'One pointer cut along its axis, half agent blue and half coral you: whoever holds it, it is one pointer.' },
    { key: 'pixel', name: 'Pixel pointer', idea: 'The pointer everyone knows, drawn in pixels like the screen it streams, its last pixel coral. Sharp even at 16 px.' },
  ] as const;

  // The classic pointer, tip at (14, 6), 31 wide and 51 tall.
  const POINTER = 'M14 6V50L24 41L31 57L38 54L31 38H45Z';
  // The pixel pointer's runs as [row, first cell, cells], 6 units a cell from (14, 2); the last run is coral.
  const PIXELS: [number, number, number][] = [
    [0, 0, 1], [1, 0, 2], [2, 0, 3], [3, 0, 4], [4, 0, 5], [5, 0, 6], [6, 0, 4], [7, 0, 1], [7, 2, 2], [8, 3, 2], [9, 3, 2],
  ];
</script>

{#snippet mark(key: string, on: Ground, size: string)}
  {@const dark = on === 'navy'}
  {@const blue = dark ? '#ffffff' : '#2457F5'}
  {@const deep = dark ? '#EAF0FF' : '#1A45D6'}
  {@const cut = dark ? '#14284B' : '#ffffff'}
  {#if key === 'duo'}
    <svg viewBox="0 0 64 64" class={size} aria-hidden="true">
      <path d={POINTER} transform="translate(30 14) scale(0.82) translate(-14 -6)" fill="#F06445" stroke-linejoin="round" />
      <path d={POINTER} transform="translate(10 4) scale(0.92) translate(-14 -6)" fill={blue} stroke={cut} stroke-width="4" stroke-linejoin="round" paint-order="stroke" />
    </svg>
  {:else if key === 'click'}
    <svg viewBox="0 0 64 64" class={size} aria-hidden="true">
      <path d="M21 21H10M28 14V3M23 16 15 8" stroke="#F06445" stroke-width="5" stroke-linecap="round" />
      <path d={POINTER} transform="translate(29 21) scale(0.82) translate(-14 -6)" fill={blue} stroke={blue} stroke-width="2.5" stroke-linejoin="round" />
    </svg>
  {:else if key === 'dash'}
    <svg viewBox="0 0 64 64" class={size} aria-hidden="true">
      <path d="M4 22H18M9 31H21M4 40H18" stroke={deep} stroke-width="4.5" stroke-linecap="round" opacity="0.55" />
      <path d="M12 31H21" stroke="#F06445" stroke-width="4.5" stroke-linecap="round" />
      <path d={POINTER} transform="translate(29 6) rotate(-12) scale(0.9) translate(-14 -6)" fill={blue} stroke={blue} stroke-width="2.5" stroke-linejoin="round" />
    </svg>
  {:else if key === 'tile'}
    <svg viewBox="0 0 64 64" class={size} aria-hidden="true">
      <rect x="2" y="2" width="60" height="60" rx="16" fill={dark ? '#2457F5' : '#2457F5'} />
      <path d={POINTER} transform="translate(21 11) scale(0.8) translate(-14 -6)" fill="#ffffff" stroke="#ffffff" stroke-width="2.5" stroke-linejoin="round" />
      <circle cx="48" cy="16" r="5.5" fill="#F06445" />
    </svg>
  {:else if key === 'split'}
    <svg viewBox="0 0 64 64" class={size} aria-hidden="true">
      <defs><clipPath id="logo-split-left"><path d="M0 0H14L36 64H0Z" /></clipPath></defs>
      <g transform="translate(6 1) scale(1.1) translate(-6 -1)" stroke-linejoin="round">
        <path d={POINTER} fill="#F06445" stroke="#F06445" stroke-width="2.5" />
        <path d={POINTER} fill={blue} stroke={blue} stroke-width="2.5" clip-path="url(#logo-split-left)" />
      </g>
    </svg>
  {:else}
    <svg viewBox="0 0 64 64" class={size} aria-hidden="true" shape-rendering="crispEdges">
      {#each PIXELS as [row, from, cells], i (i)}
        <rect x={14 + from * 6} y={2 + row * 6} width={cells * 6} height="6" fill={i === PIXELS.length - 1 ? '#F06445' : blue} />
      {/each}
    </svg>
  {/if}
{/snippet}

{#snippet word(on: Ground)}
  <span class="font-display text-[2.5rem] font-bold tracking-[-0.045em] {on === 'navy' ? 'text-white' : 'text-ink'}"
    >easi<span class="text-you">.</span>run</span
  >
{/snippet}

<main class="mx-auto max-w-[72rem] px-6 py-16">
  <p class="text-xs font-semibold uppercase tracking-[0.18em] text-accent">easi.run · logo proposals · round 4</p>
  <h1 class="mt-3 text-[clamp(2rem,4vw,3rem)] font-semibold tracking-[-0.035em] text-ink">The pointer, the agent’s and yours</h1>
  <p class="mt-3 max-w-2xl text-muted">
    Six marks built on the mouse pointer, all in the current palette, on white, on wash and on navy, with the mark at
    64, 32 and 16 pixels. The current logo is first, for reference. This page is unlisted and changes nothing on the
    landing.
  </p>
  <div class="mt-5 flex flex-wrap gap-2">
    {#each palette as color (color.hex)}
      <span class="flex items-center gap-1.5 rounded-full border border-line py-0.5 pl-0.5 pr-2.5 text-xs text-muted"
        ><span class="size-4 rounded-full ring-1 ring-black/10" style:background={color.hex}></span>{color.name}</span
      >
    {/each}
  </div>

  <section class="mt-10 rounded-2xl border border-line p-6">
    <p class="text-sm font-semibold text-muted">Current</p>
    <div class="mt-4"><Wordmark class="text-[2.5rem] text-ink" mark /></div>
  </section>

  {#each options as option, i (option.key)}
    <section class="mt-8 rounded-2xl border border-line p-6">
      <div class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <span class="grid size-8 place-items-center rounded-full bg-accent-soft text-sm font-semibold text-accent">{i + 1}</span>
        <h2 class="text-xl font-semibold tracking-tight text-ink">{option.name}</h2>
        <p class="basis-full text-muted md:basis-auto">{option.idea}</p>
      </div>
      <div class="mt-6 grid gap-4 lg:grid-cols-3">
        {#each grounds as on (on)}
          <div class="flex h-36 items-center justify-center gap-3 rounded-xl px-4 {ground[on]}">
            {@render mark(option.key, on, 'size-14 shrink-0')}
            {@render word(on)}
          </div>
        {/each}
      </div>
      <div class="mt-4 flex items-end gap-4">
        {#each ['size-16', 'size-8', 'size-4'] as size (size)}
          <div class="rounded-lg border border-line bg-paper p-2">{@render mark(option.key, 'paper', size)}</div>
        {/each}
        <div class="rounded-lg bg-ink p-2">{@render mark(option.key, 'navy', 'size-8')}</div>
      </div>
    </section>
  {/each}
</main>
