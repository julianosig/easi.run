// When the person last pressed a key or the pointer on this page: the demo starts the next visit on its own only for
// someone who was using the last one.

let last = 0;
for (const kind of ['pointerdown', 'keydown'] as const) {
  addEventListener(kind, () => (last = Date.now()), { capture: true, passive: true });
}

/** Whether the person pressed something within the last `ms` milliseconds. */
export function activeWithin(ms: number): boolean {
  return Date.now() - last < ms;
}
