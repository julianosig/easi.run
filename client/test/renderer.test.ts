import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createRenderer } from '../src/renderer.ts';

// A Canvas 2D page (no WebGL2) whose first frame's conversion to RGBA is slow, or never ends, as a stalled `copyTo`.

class FakeFrame {
  readonly displayWidth = 4;
  readonly displayHeight = 2;
  copies = 0;
  closed = false;
  constructor(private readonly copy: () => Promise<void>) {}
  copyTo(): Promise<void> {
    this.copies += 1;
    return this.copy();
  }
  close(): void {
    this.closed = true;
  }
}

function canvas2d() {
  globalThis.requestAnimationFrame = () => 1;
  globalThis.cancelAnimationFrame = () => {};
  const context = { fillRect() {}, putImageData() {}, save() {}, beginPath() {}, rect() {}, clip() {}, drawImage() {}, restore() {} };
  const canvas = { width: 0, height: 0, getContext: (kind: string) => (kind === '2d' ? context : null) };
  const renderer = createRenderer(canvas as unknown as HTMLCanvasElement, () => {});
  renderer.resize(4, 2);
  return renderer;
}

/** A first frame whose conversion ends only when `release` is called (never, if it is not), then 30 more. */
function frames(first: Promise<void>): FakeFrame[] {
  return [new FakeFrame(() => first), ...Array.from({ length: 30 }, () => new FakeFrame(() => Promise.resolve()))];
}

test('a slow conversion leaves later frames drawn whole, in order, with no buffer of their own', async () => {
  const renderer = canvas2d();
  let release!: () => void;
  const all = frames(new Promise<void>((resolve) => (release = resolve)));
  const drawn: number[] = [];
  all.forEach((frame, i) => renderer.drawVideo(frame as unknown as VideoFrame, () => null, () => drawn.push(i)));
  // Only the frames up to the queue's bound convert; the rest wait for their turn without pixels.
  assert.equal(all.reduce((sum, f) => sum + f.copies, 0), 2);
  release();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(drawn, all.slice(0, -2).map((_, i) => i));
  assert(all.slice(0, -2).every((f) => f.closed));
  renderer.destroy();
  assert(all.every((f) => f.closed));
});

test('a conversion that never ends holds the frames after it only for a while, and none once the renderer is gone', async () => {
  const renderer = canvas2d();
  const all = frames(new Promise<void>(() => {}));
  const drawn: number[] = [];
  all.forEach((frame, i) => renderer.drawVideo(frame as unknown as VideoFrame, () => null, () => drawn.push(i)));
  await new Promise((resolve) => setTimeout(resolve, 700));
  assert.deepEqual(drawn, all.slice(0, -2).map((_, i) => i));
  // More frames that never convert either, in two bursts a deadline apart: a stalled conversion still holds its
  // buffer past its deadline, so no more than two are ever under way.
  const stalled = () => Array.from({ length: 30 }, () => new FakeFrame(() => new Promise<void>(() => {})));
  const more = [...stalled(), ...stalled()];
  more.slice(0, 30).forEach((frame) => renderer.drawVideo(frame as unknown as VideoFrame, () => null, () => {}));
  await new Promise((resolve) => setTimeout(resolve, 700));
  more.slice(30).forEach((frame) => renderer.drawVideo(frame as unknown as VideoFrame, () => null, () => {}));
  assert(1 + more.filter((f) => f.copies > 0).length <= 2);
  renderer.destroy();
  assert(all.every((f) => f.closed));
});
