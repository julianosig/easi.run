// Composition on one canvas. WebGL2 by default: a persistent RGBA texture holds the whole screen; tiles update it with
// texSubImage2D and video frames are drawn into it (scissored to the parts the ordering allows); a changed screen is
// presented on the next animation frame. When the browser would run WebGL on a software rasterizer (no GPU, or a
// blocklisted one), a Canvas 2D context does the same work instead: its CPU paths for video frames and pixel writes are
// far cheaper than an emulated GPU.

import { decodeQoi } from './qoi.ts';
import type { Rect } from './protocol.ts';

const VERTEX = `#version 300 es
in vec2 pos;
uniform bool flip;
out vec2 uv;
void main() {
  uv = vec2(pos.x, flip ? 1.0 - pos.y : pos.y);
  gl_Position = vec4(pos * 2.0 - 1.0, 0.0, 1.0);
}`;

const FRAGMENT = `#version 300 es
precision mediump float;
uniform sampler2D image;
in vec2 uv;
out vec4 color;
void main() {
  color = vec4(texture(image, uv).rgb, 1.0);
}`;

interface Renderer {
  /** `webgl2`, or `canvas2d` without a hardware-accelerated WebGL2. */
  readonly kind: 'webgl2' | 'canvas2d';
  /** Measurement hook: called right after each draw, while the drawn pixels can still be read. */
  onPresent: ((read: (x: number, y: number, w: number, h: number) => Uint8Array) => void) | null;
  /** Sizes the canvas to the configuration and clears it to black. */
  resize(width: number, height: number): void;
  /** Writes one lossless tile's RGBA pixels (exactly `rect.w * rect.h * 4` bytes). */
  drawTile(rect: Rect, rgba: Uint8Array<ArrayBuffer>): void;
  /**
   * Draws a decoded frame, taking ownership of it. `place` gives its placement when it is drawn (null: nothing of it
   * is still current), so tiles drawn while it was being prepared win; `drawn` follows the draw.
   */
  drawVideo(frame: VideoFrame, place: () => Placement | null, drawn: (placed: boolean) => void): void;
  /** Frames presented since the last call. */
  takePresented(): number;
  destroy(): void;
}

/** Where a video frame goes: scaled to `rect`, drawn only inside `parts`. */
interface Placement {
  rect: Rect;
  parts: readonly Rect[];
}

/** WebGL2 when the browser runs it without a major performance caveat, else Canvas 2D. */
export function createRenderer(canvas: HTMLCanvasElement, onLost: () => void): Renderer {
  const options = { alpha: false, antialias: false, depth: false, desynchronized: true };
  const gl = canvas.getContext('webgl2', { ...options, failIfMajorPerformanceCaveat: true });
  if (gl !== null) return new GlRenderer(canvas, gl, onLost);
  const context = canvas.getContext('2d', { alpha: false, desynchronized: true });
  if (context === null) throw new Error('neither WebGL2 nor Canvas 2D is available');
  return new CanvasRenderer(canvas, context);
}

/** Presents a changed screen once per animation frame and counts what was presented. */
abstract class Presenter {
  private dirty = false;
  private frame = 0;
  private presented = 0;
  protected width = 0;
  protected height = 0;
  onPresent: ((read: (x: number, y: number, w: number, h: number) => Uint8Array) => void) | null = null;
  protected invalidate(): void {
    this.dirty = true;
    if (this.frame === 0) this.frame = requestAnimationFrame(() => this.present());
  }

  private present(): void {
    this.frame = 0;
    if (!this.dirty || !this.show()) return;
    this.dirty = false;
    this.presented++;
    this.onPresent?.((x, y, w, h) => this.read(x, y, w, h));
    if (this.waiting()) this.invalidate();
  }

  /** Whether more is waiting for the next animation frame. */
  protected waiting(): boolean {
    return false;
  }

  /** Puts the screen on the canvas; false when there is no screen yet. */
  protected abstract show(): boolean;

  /** Draws a frame where it belongs now, if anywhere, and closes it. */
  drawVideo(frame: VideoFrame, place: () => Placement | null, drawn: (placed: boolean) => void): void {
    const placement = place();
    if (placement !== null) this.drawPlaced(frame, placement);
    frame.close();
    drawn(placement !== null);
  }

  /** Draws a frame scaled to its placement's rectangle, only inside its parts. */
  protected abstract drawPlaced(frame: VideoFrame, placement: Placement): void;

  /** RGBA pixels of a drawn rectangle, top row first. */
  protected abstract read(x: number, y: number, w: number, h: number): Uint8Array;

  takePresented(): number {
    const n = this.presented;
    this.presented = 0;
    return n;
  }

  protected stop(): void {
    if (this.frame !== 0) cancelAnimationFrame(this.frame);
    this.frame = 0;
  }
}

/** Video frames that may wait for an animation frame in the Canvas 2D renderer. */
const MAX_QUEUED = 2;

interface Queued {
  frame: VideoFrame;
  buffer: Uint8ClampedArray<ArrayBuffer>;
  place: () => Placement | null;
  drawn: (placed: boolean) => void;
}

class CanvasRenderer extends Presenter implements Renderer {
  readonly kind = 'canvas2d';

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly context: CanvasRenderingContext2D,
  ) {
    super();
  }

  resize(width: number, height: number): void {
    this.canvas.width = width;
    this.canvas.height = height;
    this.width = width;
    this.height = height;
    this.context.fillStyle = '#000';
    this.context.fillRect(0, 0, width, height);
    this.invalidate();
  }

  drawTile(rect: Rect, rgba: Uint8Array<ArrayBuffer>): void {
    const pixels = new ImageData(new Uint8ClampedArray(rgba.buffer, rgba.byteOffset, rgba.byteLength), rect.w, rect.h);
    this.context.putImageData(pixels, rect.x, rect.y);
    this.invalidate();
  }

  /**
   * Converts the frame to RGBA with `copyTo`, which the browser does without holding the page's thread, then writes
   * the current parts with `putImageData`: on a software canvas this is about a third cheaper than `drawImage` and
   * leaves the thread free for the next messages meanwhile. A frame that must be scaled, or a browser that cannot
   * convert, takes `drawImage`.
   */
  override drawVideo(frame: VideoFrame, place: () => Placement | null, drawn: (placed: boolean) => void): void {
    const { displayWidth: w, displayHeight: h } = frame;
    if (this.converts === false) {
      super.drawVideo(frame, place, drawn);
      return;
    }
    const pixels = this.buffers.pop() ?? new Uint8ClampedArray(w * h * 4);
    const buffer = pixels.length === w * h * 4 ? pixels : new Uint8ClampedArray(w * h * 4);
    frame.copyTo(buffer, { format: 'RGBA', colorSpace: 'srgb' }).then(
      () => {
        this.converts = true;
        this.queue.push({ frame, buffer, place, drawn });
        // A frame still waiting when two newer ones are ready is drawn at once and never shown on its own.
        while (this.queue.length > MAX_QUEUED) this.put(this.queue.shift()!);
        this.invalidate();
      },
      () => {
        this.converts = false;
        super.drawVideo(frame, place, drawn);
      },
    );
  }

  /** Whether `copyTo` converts to RGBA here (null until the first frame says). */
  private converts: boolean | null = null;
  private readonly buffers: Uint8ClampedArray<ArrayBuffer>[] = [];
  /**
   * Converted frames waiting for an animation frame. One is drawn per animation frame, so two frames that arrive
   * within one are both shown instead of the first being overwritten unseen; at most `MAX_QUEUED` wait.
   */
  private readonly queue: Queued[] = [];

  /** Draws a converted frame where it still belongs and recycles its buffer. */
  private put({ frame, buffer, place, drawn }: Queued): void {
    const { displayWidth: w, displayHeight: h } = frame;
    const placement = place();
    if (placement !== null && (placement.rect.w !== w || placement.rect.h !== h)) {
      this.drawPlaced(frame, placement);
    } else if (placement !== null) {
      const image = new ImageData(buffer, w, h);
      const { x, y } = placement.rect;
      for (const p of placement.parts) this.context.putImageData(image, x, y, p.x - x, p.y - y, p.w, p.h);
    }
    frame.close();
    if (this.buffers.length < MAX_QUEUED + 1) this.buffers.push(buffer);
    drawn(placement !== null);
  }

  protected override waiting(): boolean {
    return this.queue.length > 0;
  }

  protected drawPlaced(frame: VideoFrame, { rect, parts }: Placement): void {
    if (parts.length === 0) return;
    const context = this.context;
    context.save();
    context.beginPath();
    for (const part of parts) context.rect(part.x, part.y, part.w, part.h);
    context.clip();
    context.drawImage(frame, rect.x, rect.y, rect.w, rect.h);
    context.restore();
    this.invalidate();
  }

  // The canvas already shows what was drawn; the oldest waiting video frame joins it now. A burst is absorbed by
  // showing its frames on successive animation frames, but a backlog still there on the next one is drawn at once,
  // so the screen never stays a frame behind.
  protected show(): boolean {
    const next = this.queue.shift();
    if (next !== undefined) this.put(next);
    this.backlog = this.queue.length > 0 ? this.backlog + 1 : 0;
    if (this.backlog >= 2) {
      for (const queued of this.queue.splice(0)) this.put(queued);
      this.backlog = 0;
    }
    return this.width > 0;
  }

  /** Animation frames in a row that ended with frames still waiting. */
  private backlog = 0;

  protected read(x: number, y: number, w: number, h: number): Uint8Array {
    const data = this.context.getImageData(x, y, w, h).data;
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  }

  destroy(): void {
    this.stop();
    for (const { frame } of this.queue.splice(0)) frame.close();
  }
}

class GlRenderer extends Presenter implements Renderer {
  readonly kind = 'webgl2';
  private readonly gl: WebGL2RenderingContext;
  private readonly program: WebGLProgram;
  private readonly flip: WebGLUniformLocation;
  private readonly video: WebGLTexture;
  private readonly target: WebGLFramebuffer;
  private screen: WebGLTexture | null = null;

  constructor(
    readonly canvas: HTMLCanvasElement,
    gl: WebGL2RenderingContext,
    onLost: () => void,
  ) {
    super();
    this.gl = gl;
    canvas.addEventListener('webglcontextlost', (e) => {
      e.preventDefault();
      onLost();
    });
    this.program = link(gl);
    this.flip = gl.getUniformLocation(this.program, 'flip')!;
    const quad = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, quad);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
    const vao = gl.createVertexArray();
    gl.bindVertexArray(vao);
    const pos = gl.getAttribLocation(this.program, 'pos');
    gl.enableVertexAttribArray(pos);
    gl.vertexAttribPointer(pos, 2, gl.FLOAT, false, 0, 0);
    gl.useProgram(this.program);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    this.video = texture(gl, gl.LINEAR);
    this.target = gl.createFramebuffer();
  }

  resize(width: number, height: number): void {
    const gl = this.gl;
    this.canvas.width = width;
    this.canvas.height = height;
    this.width = width;
    this.height = height;
    if (this.screen !== null) gl.deleteTexture(this.screen);
    this.screen = texture(gl, gl.NEAREST);
    gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA8, width, height);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.target);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.screen, 0);
    gl.clearColor(0, 0, 0, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    this.invalidate();
  }

  drawTile(rect: Rect, rgba: Uint8Array<ArrayBuffer>): void {
    const gl = this.gl;
    gl.bindTexture(gl.TEXTURE_2D, this.screen);
    gl.texSubImage2D(gl.TEXTURE_2D, 0, rect.x, rect.y, rect.w, rect.h, gl.RGBA, gl.UNSIGNED_BYTE, rgba);
    this.invalidate();
  }

  protected drawPlaced(frame: VideoFrame, { rect, parts }: Placement): void {
    if (parts.length === 0) return;
    const gl = this.gl;
    gl.bindTexture(gl.TEXTURE_2D, this.video);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, frame);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.target);
    gl.viewport(rect.x, rect.y, rect.w, rect.h);
    gl.uniform1i(this.flip, 0);
    gl.enable(gl.SCISSOR_TEST);
    for (const part of parts) {
      gl.scissor(part.x, part.y, part.w, part.h);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    }
    gl.disable(gl.SCISSOR_TEST);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    this.invalidate();
  }

  protected show(): boolean {
    if (this.screen === null) return false;
    const gl = this.gl;
    gl.viewport(0, 0, this.width, this.height);
    gl.bindTexture(gl.TEXTURE_2D, this.screen);
    gl.uniform1i(this.flip, 1);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    return true;
  }

  protected read(x: number, y: number, w: number, h: number): Uint8Array {
    const gl = this.gl;
    const bottomUp = new Uint8Array(w * h * 4);
    gl.readPixels(x, this.height - y - h, w, h, gl.RGBA, gl.UNSIGNED_BYTE, bottomUp);
    const out = new Uint8Array(w * h * 4);
    for (let row = 0; row < h; row++) {
      out.set(bottomUp.subarray((h - 1 - row) * w * 4, (h - row) * w * 4), row * w * 4);
    }
    return out;
  }

  destroy(): void {
    this.stop();
    this.gl.getExtension('WEBGL_lose_context')?.loseContext();
  }
}

function texture(gl: WebGL2RenderingContext, filter: number): WebGLTexture {
  const t = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, t);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  return t;
}

function link(gl: WebGL2RenderingContext): WebGLProgram {
  const program = gl.createProgram();
  for (const [type, source] of [
    [gl.VERTEX_SHADER, VERTEX],
    [gl.FRAGMENT_SHADER, FRAGMENT],
  ] as const) {
    const shader = gl.createShader(type)!;
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    gl.attachShader(program, shader);
  }
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw new Error(`WebGL program: ${gl.getProgramInfoLog(program) ?? 'link failed'}`);
  }
  return program;
}

/**
 * Cursor shapes by number: 0 hides the pointer and 1..34 follow the `wp_cursor_shape_device_v1` shape enum, whose
 * names are the CSS cursor keywords.
 */
const SHAPES = [
  'none', 'default', 'context-menu', 'help', 'pointer', 'progress', 'wait', 'cell', 'crosshair', 'text',
  'vertical-text', 'alias', 'copy', 'move', 'no-drop', 'not-allowed', 'grab', 'grabbing', 'e-resize', 'n-resize',
  'ne-resize', 'nw-resize', 's-resize', 'se-resize', 'sw-resize', 'w-resize', 'ew-resize', 'ns-resize',
  'nesw-resize', 'nwse-resize', 'col-resize', 'row-resize', 'all-scroll', 'zoom-in', 'zoom-out',
] as const;

/** Largest decoded cursor image, in pixels. */
const MAX_CURSOR_PIXELS = 256 * 256;

/**
 * The remote pointer: a named shape becomes the CSS cursor over the screen; an image is drawn in a small overlay
 * canvas at the pointer's position (the hotspot on the position), scaled like the screen.
 */
export class Cursor {
  private readonly overlay = document.createElement('canvas');
  private image: { hotX: number; hotY: number } | null = null;
  private x = 0;
  private y = 0;

  constructor(
    private readonly screen: HTMLCanvasElement,
    private readonly surface: HTMLElement,
  ) {
    this.overlay.style.cssText = 'position:absolute;left:0;top:0;pointer-events:none;display:none;image-rendering:pixelated';
    screen.insertAdjacentElement('afterend', this.overlay);
  }

  /** Applies one cursor message; a malformed image throws. */
  update(x: number, y: number, shape: number, hotX: number, hotY: number, image: Uint8Array): void {
    this.x = x;
    this.y = y;
    if (image.length === 0) {
      this.image = null;
      this.overlay.style.display = 'none';
      this.surface.style.cursor = SHAPES[shape] ?? 'default';
      return;
    }
    const decoded = decodeQoi(image, { maxPixels: MAX_CURSOR_PIXELS });
    this.overlay.width = decoded.width;
    this.overlay.height = decoded.height;
    const pixels = new ImageData(new Uint8ClampedArray(decoded.rgba.buffer), decoded.width, decoded.height);
    this.overlay.getContext('2d')?.putImageData(pixels, 0, 0);
    this.image = { hotX, hotY };
    this.surface.style.cursor = 'none';
    this.overlay.style.display = 'block';
    this.place();
  }

  /** Repositions the overlay after the pointer moved or the screen was resized on the page. */
  place(): void {
    if (this.image === null || this.screen.width === 0) return;
    const scale = this.screen.clientWidth / this.screen.width;
    const s = this.overlay.style;
    s.width = `${this.overlay.width * scale}px`;
    s.height = `${this.overlay.height * scale}px`;
    s.transform = `translate(${this.screen.offsetLeft + (this.x - this.image.hotX) * scale}px, ${
      this.screen.offsetTop + (this.y - this.image.hotY) * scale
    }px)`;
  }

  reset(): void {
    this.image = null;
    this.overlay.style.display = 'none';
    this.surface.style.cursor = '';
  }

  destroy(): void {
    this.overlay.remove();
    this.surface.style.cursor = '';
  }
}
