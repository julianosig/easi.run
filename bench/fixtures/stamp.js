// A machine-readable clock in the top-left corner: 44 blocks of 16x16 pixels, white-black sentinels, the low 40 bits
// of Date.now() (most significant first, white = 1), and black-white sentinels. A client decodes it from a drawn
// frame and subtracts it from its own clock (the same host clock in the benchmark) to estimate capture-to-draw.
const BLOCK = 16;
const BITS = 40;
/** Draws the strip of time `t` (ms) at the top-left corner of the canvas `ctx` draws on. */
export function drawStamp(ctx, t) {
  const bits = [1, 0];
  for (let i = BITS - 1; i >= 0; i--) bits.push(Math.floor(t / 2 ** i) % 2);
  bits.push(0, 1);
  bits.forEach((bit, n) => {
    ctx.fillStyle = bit ? '#fff' : '#000';
    ctx.fillRect(n * BLOCK, 0, BLOCK, BLOCK);
  });
}

/** Keeps the strip of the current time over the page, on a canvas of its own, until `stopAt` (`performance.now()`). */
export function startStamp(stopAt = Infinity) {
  const canvas = document.createElement('canvas');
  canvas.width = (BITS + 4) * BLOCK;
  canvas.height = BLOCK;
  canvas.style.cssText = 'position:fixed;left:0;top:0;z-index:2147483647;image-rendering:pixelated';
  document.body.append(canvas);
  const ctx = canvas.getContext('2d');
  const draw = () => {
    drawStamp(ctx, Date.now());
    if (performance.now() <= stopAt) requestAnimationFrame(draw);
  };
  requestAnimationFrame(draw);
}
