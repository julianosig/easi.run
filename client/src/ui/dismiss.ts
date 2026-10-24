// The way every popover closes: a press anywhere outside it, or Escape (which puts focus back on its first button,
// the one that opened it, since the focused item goes away with it). A press inside `[data-keep-open]` (the camera
// preview, dragged into place while the Record menu chooses its look) leaves it open.

/** Listens while a popover rooted at `root` is open; returns the cleanup, for the `$effect` that opened it. */
export function dismissable(root: HTMLElement | undefined, close: () => void): () => void {
  const away = (e: Event) => {
    const target = e.target as Element;
    if (root && !root.contains(target) && !target.closest?.('[data-keep-open]')) close();
  };
  const key = (e: KeyboardEvent) => {
    if (e.key !== 'Escape') return;
    close();
    root?.querySelector('button')?.focus();
  };
  // Captured, so a press the screen or another control stops on its way still closes it.
  document.addEventListener('pointerdown', away, true);
  document.addEventListener('keydown', key);
  return () => {
    document.removeEventListener('pointerdown', away, true);
    document.removeEventListener('keydown', key);
  };
}
