// A QR code drawn by the page itself (uqr, MIT, without dependencies), so the authenticator app's secret never leaves
// the browser for an image service.

import { encode } from 'uqr';

/** The quiet zone around the code, in modules: the four the QR standard asks for, which phone cameras need. */
const QUIET = 4;

/**
 * The QR code of `text` as one SVG path of unit squares, one per dark module, and its width in modules, quiet zone
 * included. Medium error correction: a phone reads it off a screen with glare or a smudge.
 */
export function qrPath(text: string): { size: number; path: string } {
  const { data, size } = encode(text, { ecc: 'M', border: QUIET });
  let path = '';
  data.forEach((row, y) =>
    row.forEach((dark, x) => {
      if (dark) path += `M${x} ${y}h1v1h-1z`;
    }),
  );
  return { size, path };
}
