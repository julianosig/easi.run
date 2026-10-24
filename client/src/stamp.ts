// Decodes the fixtures' time strip (bench/fixtures/stamp.js): 44 blocks of 16x16 pixels at the top-left corner,
// white-black sentinels, the low 40 bits of Date.now() most significant first, black-white sentinels.

export const STAMP_WIDTH = 44 * 16;
export const STAMP_HEIGHT = 16;

const SPAN = 2 ** 40;

/** The full time of a stamp's low 40 bits, given a nearby `now` (within 2^40 ms, about 35 years). */
export function unwrapStamp(low: number, now: number): number {
  return now - (((now % SPAN) - low + SPAN) % SPAN);
}

/** The low 40 bits of the time stamp in an RGBA strip read from the top-left `STAMP_WIDTH` x `STAMP_HEIGHT` pixels. */
export function decodeStamp(rgba: Uint8Array, stride = STAMP_WIDTH): number | null {
  const bit = (n: number): number | null => {
    const i = (8 * stride + n * 16 + 8) * 4;
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
