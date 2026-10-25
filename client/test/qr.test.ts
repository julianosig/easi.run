import assert from 'node:assert/strict';
import { test } from 'node:test';
import { qrPath } from '../src/ui/qr.ts';

/** The dark modules a path draws, as `x,y`. */
function modules(path: string): Set<string> {
  return new Set([...path.matchAll(/M(\d+) (\d+)h1v1h-1z/g)].map((m) => `${m[1]},${m[2]}`));
}

test('a QR code has its three finder patterns inside a quiet zone of four modules', () => {
  const uri = 'otpauth://totp/easi.run:admin?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=easi.run';
  const { size, path } = qrPath(uri);
  const dark = modules(path);
  const inner = size - 8;
  assert((inner - 17) % 4 === 0 && inner >= 21, `${inner} modules is a QR version's size`);
  // A finder pattern: a dark 7x7 ring, a light ring, a dark 3x3 centre.
  const finder = (left: number, top: number) => {
    for (let y = 0; y < 7; y++) {
      for (let x = 0; x < 7; x++) {
        const ring = x === 0 || y === 0 || x === 6 || y === 6;
        const centre = x >= 2 && x <= 4 && y >= 2 && y <= 4;
        assert.equal(dark.has(`${left + x},${top + y}`), ring || centre, `${left + x},${top + y}`);
      }
    }
  };
  finder(4, 4);
  finder(size - 11, 4);
  finder(4, size - 11);
  for (const [x, y] of [...dark].map((m) => m.split(',').map(Number))) {
    assert(x! >= 4 && y! >= 4 && x! < size - 4 && y! < size - 4, 'nothing in the quiet zone');
  }
  assert.equal(qrPath(uri).path, path, 'the same text, the same code');
});
