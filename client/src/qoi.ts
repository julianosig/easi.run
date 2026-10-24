// A bounded QOI decoder (https://qoiformat.org/qoi-specification.pdf) producing straight-alpha RGBA bytes.
//
// It is strict where the specification lets decoders be lenient: the pixel stream must produce exactly
// width * height pixels, a run may not cross the last pixel, and the 8-byte end marker must follow immediately and
// end the input.

const MAX_QOI_PIXELS = 4096 * 4096;

interface Image {
  width: number;
  height: number;
  rgba: Uint8Array<ArrayBuffer>;
}

const HEADER = 14;
const END = [0, 0, 0, 0, 0, 0, 0, 1];
const OP_INDEX = 0x00;
const OP_DIFF = 0x40;
const OP_LUMA = 0x80;
const OP_RGB = 0xfe;
const OP_RGBA = 0xff;
const MASK = 0xc0;
/** The most pixels one byte of stream can produce (a run of 62). */
const MAX_PIXELS_PER_BYTE = 62;

export class QoiError extends Error {
  override name = 'QoiError';
}

/** Decodes `bytes` as one QOI image. `maxPixels` lowers the bound (a cursor's, say). */
export function decodeQoi(bytes: Uint8Array, options: { maxPixels?: number } = {}): Image {
  if (bytes.length < HEADER + END.length) throw new QoiError('truncated');
  if (bytes[0] !== 0x71 || bytes[1] !== 0x6f || bytes[2] !== 0x69 || bytes[3] !== 0x66) throw new QoiError('magic');
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const width = view.getUint32(4);
  const height = view.getUint32(8);
  const channels = bytes[12];
  const colorspace = bytes[13];
  if (channels !== 3 && channels !== 4) throw new QoiError('channels');
  if (colorspace !== 0 && colorspace !== 1) throw new QoiError('colorspace');
  if (width === 0 || height === 0) throw new QoiError('empty image');
  const pixels = width * height;
  if (pixels > Math.min(MAX_QOI_PIXELS, options.maxPixels ?? MAX_QOI_PIXELS)) throw new QoiError('too large');
  const streamEnd = bytes.length - END.length;
  // Refuse a header that claims more pixels than the stream could possibly hold before allocating for it.
  if (pixels > (streamEnd - HEADER) * MAX_PIXELS_PER_BYTE) throw new QoiError('truncated');

  const out = new Uint8Array(pixels * 4);
  const index = new Uint8Array(64 * 4);
  let r = 0;
  let g = 0;
  let b = 0;
  let a = 255;
  let p = HEADER;
  const need = (n: number): void => {
    if (p + n > streamEnd) throw new QoiError('truncated');
  };
  for (let px = 0; px < pixels; ) {
    need(1);
    const op = bytes[p++]!;
    let run = 1;
    if (op === OP_RGB) {
      need(3);
      r = bytes[p]!;
      g = bytes[p + 1]!;
      b = bytes[p + 2]!;
      p += 3;
    } else if (op === OP_RGBA) {
      need(4);
      r = bytes[p]!;
      g = bytes[p + 1]!;
      b = bytes[p + 2]!;
      a = bytes[p + 3]!;
      p += 4;
    } else if ((op & MASK) === OP_INDEX) {
      const at = op * 4;
      r = index[at]!;
      g = index[at + 1]!;
      b = index[at + 2]!;
      a = index[at + 3]!;
    } else if ((op & MASK) === OP_DIFF) {
      r = (r + ((op >> 4) & 3) - 2) & 0xff;
      g = (g + ((op >> 2) & 3) - 2) & 0xff;
      b = (b + (op & 3) - 2) & 0xff;
    } else if ((op & MASK) === OP_LUMA) {
      need(1);
      const second = bytes[p++]!;
      const dg = (op & 0x3f) - 32;
      r = (r + dg + (second >> 4) - 8) & 0xff;
      g = (g + dg) & 0xff;
      b = (b + dg + (second & 0x0f) - 8) & 0xff;
    } else {
      // OP_RUN: the low six bits hold run - 1 (62 and 63 are the RGB and RGBA tags handled above).
      run = (op & 0x3f) + 1;
      if (px + run > pixels) throw new QoiError('run past the last pixel');
    }
    const at = ((r * 3 + g * 5 + b * 7 + a * 11) % 64) * 4;
    index[at] = r;
    index[at + 1] = g;
    index[at + 2] = b;
    index[at + 3] = a;
    for (const end = px + run; px < end; px++) {
      const o = px * 4;
      out[o] = r;
      out[o + 1] = g;
      out[o + 2] = b;
      out[o + 3] = a;
    }
  }
  if (p !== streamEnd) throw new QoiError('trailing bytes');
  for (let i = 0; i < END.length; i++) {
    if (bytes[streamEnd + i] !== END[i]) throw new QoiError('end marker');
  }
  return { width, height, rgba: out };
}
