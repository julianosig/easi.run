// Waiting for something, at most so long.

/**
 * Waits for `work`, at most `ms`: past that, `late` is called and its value is the answer. Its timer goes once `work`
 * settled, so a late call never reaches what came after it (a recording started since, numbered as the one it waited
 * for).
 */
export function awaitAtMost<T>(work: Promise<T>, ms: number, late: () => NoInfer<T>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const expired = new Promise<T>((resolve) => {
    timer = setTimeout(() => resolve(late()), ms);
  });
  return Promise.race([work.finally(() => clearTimeout(timer)), expired]);
}
