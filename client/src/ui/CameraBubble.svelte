<script lang="ts">
  // The camera over the screen, where the recording puts it: a square the person drags anywhere (or moves with the
  // arrow keys) and resizes by its corner handle (or with + and -). Its corner radius is chosen in the recording menu.
  import { type CameraLayout, cameraRect, type KeptCorner, keptCorner, moveCamera, resizeCamera } from '../recorder.ts';
  import { focusRing } from './ui.ts';

  let {
    stream,
    layout,
    width,
    height,
    onLayout,
  }: {
    stream: MediaStream;
    layout: CameraLayout;
    /** The screen's size on the page, in CSS pixels. */
    width: number;
    height: number;
    /** A new layout; `settled` once a drag ends (while dragging, changes come often). */
    onLayout: (layout: CameraLayout, settled: boolean) => void;
  } = $props();

  let dragging = $state(false);
  let root: HTMLDivElement | undefined = $state();
  const rect = $derived(cameraRect(width, height, layout));
  // The resize keeps the corner toward the nearest screen edges; the handle is on the opposite one, toward the middle.
  // While resizing, the kept corner and the layout are the ones the press started with, so rounding never moves the corner.
  let resizing = $state<{ id: number; kept: KeptCorner; start: CameraLayout } | null>(null);
  const kept = $derived(resizing?.kept ?? keptCorner(width, height, layout));

  let drag: { id: number; dx: number; dy: number; fromX: number; fromY: number } | null = null;
  /** The press has moved the camera (a few pixels of wobble is still a press). */
  let moved = false;
  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const box = target.getBoundingClientRect();
    drag = { id: e.pointerId, dx: e.clientX - box.left, dy: e.clientY - box.top, fromX: e.clientX, fromY: e.clientY };
    moved = false;
    e.preventDefault();
  }
  function move(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.id || !root) return;
    const screen = root.parentElement?.getBoundingClientRect();
    if (!screen || width === 0 || height === 0) return;
    if (!moved && Math.hypot(e.clientX - drag.fromX, e.clientY - drag.fromY) < 4) return;
    moved = true;
    dragging = true;
    const x = (e.clientX - drag.dx - screen.left - rect.x) / width;
    const y = (e.clientY - drag.dy - screen.top - rect.y) / height;
    onLayout(moveCamera(width, height, layout, x, y), false);
  }
  function up(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.id) return;
    drag = null;
    dragging = false;
    if (moved) onLayout(layout, true);
  }

  /** The handle's press: the camera's side follows the pointer's distance from the kept corner. */
  function resizeDown(e: PointerEvent) {
    if (e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    resizing = { id: e.pointerId, kept: keptCorner(width, height, layout), start: layout };
    e.preventDefault();
    e.stopPropagation();
  }
  function resizeMove(e: PointerEvent) {
    if (!resizing || e.pointerId !== resizing.id || !root) return;
    const screen = root.parentElement?.getBoundingClientRect();
    if (!screen || width === 0 || height === 0) return;
    const { kept, start } = resizing;
    const from = cameraRect(width, height, start);
    const cornerX = kept.right ? from.x + from.w : from.x;
    const cornerY = kept.bottom ? from.y + from.h : from.y;
    const across = Math.abs(e.clientX - screen.left - cornerX);
    const along = Math.abs(e.clientY - screen.top - cornerY);
    onLayout(resizeCamera(width, height, start, Math.max(across, along) / width, kept), false);
  }
  function resizeUp(e: PointerEvent) {
    if (!resizing || e.pointerId !== resizing.id) return;
    resizing = null;
    onLayout(layout, true);
  }

  /**
   * Arrow keys move the camera by 1% of the screen (5% with Shift); + and - resize it by 2% of the screen's width
   * (`=` and `_` too, the same keys without or with Shift on many keyboards).
   */
  function key(e: KeyboardEvent) {
    const step = e.shiftKey ? 0.05 : 0.01;
    const by: Record<string, [number, number]> = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] };
    const grow: Record<string, number> = { '+': 0.02, '=': 0.02, '-': -0.02, _: -0.02 };
    const d = by[e.key];
    const g = grow[e.key];
    if (d) onLayout(moveCamera(width, height, layout, d[0], d[1]), true);
    else if (g !== undefined) onLayout(resizeCamera(width, height, layout, layout.side + g, keptCorner(width, height, layout)), true);
    else return;
    e.preventDefault();
  }

  function live(node: HTMLVideoElement, s: MediaStream) {
    node.srcObject = s;
    return { update: (next: MediaStream) => (node.srcObject = next), destroy: () => (node.srcObject = null) };
  }
</script>

<div
  bind:this={root}
  data-keep-open
  class="pointer-events-none absolute z-10"
  style="left: {rect.x}px; top: {rect.y}px; width: {rect.w}px; height: {rect.h}px"
>
  <button
    type="button"
    class="pointer-events-auto relative block size-full touch-none select-none overflow-hidden shadow-[0_18px_44px_-14px_rgb(20_40_75/0.55)] {focusRing} {dragging ? 'cursor-grabbing' : 'cursor-grab'}"
    style="border-radius: {layout.radius / 2}%"
    aria-label="Your camera: drag it or use the arrow keys to move it; drag its corner or use + and - to resize it"
    aria-keyshortcuts="ArrowLeft ArrowRight ArrowUp ArrowDown + -"
    onpointerdown={down}
    onpointermove={move}
    onpointerup={up}
    onpointercancel={up}
    onkeydown={key}
  >
    <video class="pointer-events-none size-full object-cover" muted autoplay playsinline use:live={stream}></video>
  </button>
  <!-- The resize handle, on the corner toward the middle of the screen (the keyboard resizes through the camera itself). -->
  <span
    class="pointer-events-auto absolute grid size-6 touch-none place-items-center {kept.right ? 'left-0 -translate-x-1/2' : 'right-0 translate-x-1/2'} {kept.bottom ? 'top-0 -translate-y-1/2' : 'bottom-0 translate-y-1/2'} {kept.right === kept.bottom ? 'cursor-nwse-resize' : 'cursor-nesw-resize'}"
    aria-hidden="true"
    onpointerdown={resizeDown}
    onpointermove={resizeMove}
    onpointerup={resizeUp}
    onpointercancel={resizeUp}
  >
    <span class="size-3.5 rounded-full border-2 border-accent bg-white shadow-[0_2px_6px_rgb(20_40_75/0.45)]"></span>
  </span>
</div>
