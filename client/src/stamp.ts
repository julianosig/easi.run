// Decodes the fixtures' time strip (bench/fixtures/stamp.js): 44 blocks of 16x16 pixels at the top-left corner,
// white-black sentinels, the low 40 bits of Date.now() most significant first, black-white sentinels.

export const STAMP_WIDTH = 44 * 16;
export const STAMP_HEIGHT = 16;

const SPAN = 2 ** 40;

/**
 * The full time of a stamp's low 40 bits, given a nearby `now`: the time those bits name that is nearest `now`, so one
 * within 2^39 ms (about 17 years) of it on either side. A stamp drawn a moment after `now` (the clock that drew it a few
 * milliseconds ahead of this one) is a moment in the future, not 2^40 ms in the past.
 */
export function unwrapStamp(low: number, now: number): number {
  const before = now - (((now % SPAN) - low + SPAN) % SPAN);
  return now - before > SPAN / 2 ? before + SPAN : before;
}

/** The low 40 bits of the time stamp in an RGBA strip read from the top-left `STAMP_WIDTH` x `STAMP_HEIGHT` pixels. */
export function decodeStamp(rgba: Uint8Array): number | null {
  const bit = (n: number): number | null => {
    const i = (8 * STAMP_WIDTH + n * 16 + 8) * 4;
    const level = (rgba[i]! + rgba[i + 1]! + rgba[i + 2]!) / 3;
    if (level > 192) return 1;
    if (level < 64) return 0;
    return null;
  };
  const sentinels = [bit(0), bit(1), bit(42), bit(43)];
  if (sentinels.join() !== '1,0,0,1') return null;
  let value = 0;
  for (let n = 2; n < 42; n++) {
    const b = bit(n);
    if (b === null) return null;
    value = value * 2 + b;
  }
  return value;
}
