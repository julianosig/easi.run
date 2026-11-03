/** `run` at once, then at most once every `gap` milliseconds however often it is asked for, the last ask never lost. */
export function throttled(run: () => void, gap: number): { ask: () => void; stop: () => void } {
  let last = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const go = () => {
    timer = undefined;
    last = Date.now();
    run();
  };
  return {
    ask() {
      if (timer === undefined) timer = setTimeout(go, Math.max(0, last + gap - Date.now()));
    },
    stop() {
      clearTimeout(timer);
      timer = undefined;
    },
  };
}
