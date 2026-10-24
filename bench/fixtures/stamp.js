// A machine-readable clock in the top-left corner: 44 blocks of 16x16 pixels, white-black sentinels, the low 40 bits
// of Date.now() (most significant first, white = 1), and black-white sentinels. A client decodes it from a drawn
// frame and subtracts it from its own clock (the same host clock in the benchmark) to estimate capture-to-draw.
const BLOCK = 16;
const BITS = 40;
export function startStamp() {
  const canvas = document.createElement('canvas');
  canvas.width = (BITS + 4) * BLOCK;
  canvas.height = BLOCK;
  canvas.style.cssText = 'position:fixed;left:0;top:0;z-index:2147483647;image-rendering:pixelated';
  document.body.append(canvas);
  const ctx = canvas.getContext('2d');
  const cells = (now) => {
    const out = [1, 0];
    for (let i = BITS - 1; i >= 0; i--) out.push(Math.floor(now / 2 ** i) % 2);
    return out.concat([0, 1]);
  };
  const draw = () => {
    cells(Date.now()).forEach((bit, i) => {
      ctx.fillStyle = bit ? '#fff' : '#000';
      ctx.fillRect(i * BLOCK, 0, BLOCK, BLOCK);
    });
    requestAnimationFrame(draw);
  };
  requestAnimationFrame(draw);
}
