<script lang="ts">
  // A card over the screen that a person can move off what they are working on: dragged by anything in it but its other
  // buttons and its selectable text (`select-text`), or by its grip's arrow keys. It sits where its parent puts it until
  // then, and stays in the window where it is put, remembered in this browser for the next time it shows.
  import type { Snippet } from 'svelte';
  import { remember, remembered } from './api.ts';
  import Icon from './Icon.svelte';
  import { ICONS } from './icons.ts';
  import { focusRing } from './ui.ts';

  let {
    name,
    label,
    moveLabel,
    class: className = '',
    children,
  }: {
    /** What this browser remembers the place under. */
    name: string;
    label: string;
    /** What the grip does, as its name and tooltip. */
    moveLabel: string;
    class?: string;
    children: Snippet;
  } = $props();

  /** The window's edge to the card, at least. */
  const MARGIN = 8;
  /** An arrow key's step, in CSS pixels, and with Shift. */
  const STEP = 16;
  const LONG_STEP = 64;

  type Place = { x: number; y: number };
  function kept(): Place | null {
    try {
      const { x, y } = JSON.parse(remembered(name) ?? '');
      return Number.isFinite(x) && Number.isFinite(y) ? { x, y } : null;
    } catch {
      return null;
    }
  }

  let root: HTMLDivElement | undefined = $state();
  let at: Place | null = $state(kept());
  let dragging = $state(false);
  let press: { id: number; dx: number; dy: number; fromX: number; fromY: number } | null = null;

  function put(x: number, y: number) {
    if (!root) return;
    const box = root.getBoundingClientRect();
    at = {
      x: Math.max(MARGIN, Math.min(x, window.innerWidth - box.width - MARGIN)),
      y: Math.max(MARGIN, Math.min(y, window.innerHeight - box.height - MARGIN)),
    };
  }
  const keep = () => remember(name, JSON.stringify(at));

  // A new text in the card, or a smaller window, must not leave it outside.
  $effect(() => {
    if (!root) return;
    const fit = () => {
      if (at) put(at.x, at.y);
    };
    const observer = new ResizeObserver(fit);
    observer.observe(root);
    addEventListener('resize', fit);
    return () => {
      observer.disconnect();
      removeEventListener('resize', fit);
    };
  });

  const pressable = (target: EventTarget | null) => (target as Element).closest('button:not([data-grip]), .select-text') !== null;

  function down(e: PointerEvent) {
    if (e.button !== 0 || pressable(e.target) || !root) return;
    root.setPointerCapture(e.pointerId);
    const box = root.getBoundingClientRect();
    press = { id: e.pointerId, dx: e.clientX - box.left, dy: e.clientY - box.top, fromX: e.clientX, fromY: e.clientY };
  }
  function move(e: PointerEvent) {
    if (!press || e.pointerId !== press.id) return;
    // A few pixels of wobble is still a press.
    if (!dragging && Math.hypot(e.clientX - press.fromX, e.clientY - press.fromY) < 4) return;
    dragging = true;
    put(e.clientX - press.dx, e.clientY - press.dy);
  }
  function up(e: PointerEvent) {
    if (!press || e.pointerId !== press.id) return;
    press = null;
    if (dragging) keep();
    dragging = false;
  }
  function key(e: KeyboardEvent) {
    const step = e.shiftKey ? LONG_STEP : STEP;
    const by: Record<string, [number, number]> = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] };
    const arrow = by[e.key];
    if (!arrow || !root) return;
    e.preventDefault();
    const box = root.getBoundingClientRect();
    put(box.left + arrow[0], box.top + arrow[1]);
    keep();
  }
</script>

<!-- Pressing it leaves the focus where it is: what a person holding control types still reaches the page. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions (the keyboard's way to move it is the grip) -->
<div
  bind:this={root}
  class="{className} touch-none select-none {dragging ? 'cursor-grabbing' : 'cursor-grab'} {at ? 'fixed w-max max-w-[calc(100vw-1rem)]' : ''}"
  style={at ? `left: ${at.x}px; top: ${at.y}px` : undefined}
  role="group"
  aria-label={label}
  onmousedown={(e) => {
    if (!pressable(e.target)) e.preventDefault();
  }}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
>
  <button
    data-grip
    type="button"
    class="-mx-1.5 grid h-9 w-6 shrink-0 cursor-[inherit] place-items-center rounded-lg text-muted hover:text-ink {focusRing}"
    title={moveLabel}
    aria-label={moveLabel}
    onkeydown={key}
  >
    <Icon d={ICONS.grip} class="size-[18px]" />
  </button>
  {@render children()}
</div>
