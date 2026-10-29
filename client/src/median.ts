// The middle of a measurement's samples, as the stats report it.

/** The upper median of `samples` (sorted in place), rounded; null without samples. */
export function p50(samples: number[]): number | null {
  return samples.length === 0 ? null : Math.round(samples.sort((a, b) => a - b)[samples.length >> 1]!);
}
