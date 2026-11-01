// A replay's screen: what a recording's messages draw (admin/src/replay.rs), as the live viewer draws them (the same
// ordering, decoder and renderer, see viewer.ts) but with no connection: no input, no acknowledgement, no keyframe
// to ask for. A seek starts it over (`reset`) from a segment's start, which a recording begins with a configuration
// and a whole picture.

import { AgentCursor } from './agent.ts';
import { VideoStream } from './decoder.ts';
import { StreamOrder } from './order.ts';
import { Codec, decode, type MessageOf } from './protocol.ts';
import { createRenderer, Cursor } from './renderer.ts';
import { decodeTiles } from './tile.ts';

export class ReplayScreen {
  readonly canvas = document.createElement('canvas');
  private readonly renderer: ReturnType<typeof createRenderer>;
  private readonly cursor: Cursor;
  private readonly agentCursor: AgentCursor;
  private order = new StreamOrder();
  private video: VideoStream | null = null;
  private readonly resizeObserver: ResizeObserver;

  /** `onLost`: the screen can draw no more (its WebGL context was lost); `onError`: a message would not decode. */
  constructor(
    container: HTMLElement,
    agentLabel: string,
    private readonly onError: (error: Error) => void,
  ) {
    this.canvas.style.cssText = 'display:block;max-width:100%;max-height:100%;width:auto;height:auto';
    if (getComputedStyle(container).position === 'static') container.style.position = 'relative';
    container.append(this.canvas);
    this.renderer = createRenderer(this.canvas, () => onError(new Error('the WebGL context was lost')));
    this.cursor = new Cursor(this.canvas, container);
    this.agentCursor = new AgentCursor(this.canvas, agentLabel);
    this.resizeObserver = new ResizeObserver(() => {
      this.cursor.place();
      this.agentCursor.place();
    });
    this.resizeObserver.observe(this.canvas);
  }

  /** Forgets what it drew from, for a seek: the next configuration draws afresh. */
  reset(): void {
    this.order = new StreamOrder();
    this.video?.reset();
    this.video = null;
    this.cursor.reset();
  }

  /** One recorded display message, as a viewer got it. */
  message(bytes: Uint8Array): void {
    let m;
    try {
      m = decode(bytes);
    } catch (e) {
      return this.onError(e instanceof Error ? e : new Error(String(e)));
    }
    switch (m.type) {
      case 'config':
        return this.config(m);
      case 'video':
        return this.frame(m);
      case 'tiles':
        return this.tiles(m);
      case 'cursor':
        if (this.order.cursor(m) === 'apply') this.cursor.update(m.x, m.y, m.shape, m.hotX, m.hotY, m.image);
        return;
    }
  }

  /** Where the agent acted, in the screen's pixels. */
  pointer(x: number, y: number): void {
    this.agentCursor.show(x, y);
  }

  private config(m: MessageOf<'config'>): void {
    if (this.order.config(m, performance.now()).verdict !== 'reset') return;
    this.renderer.resize(m.width, m.height);
    this.canvas.style.setProperty('--screen-ratio', String(m.width / m.height));
    this.agentCursor.configured();
    this.cursor.reset();
    this.video?.reset();
    // A frame decoded for this configuration draws only while its ordering is the screen's: after a seek's reset, one
    // still being converted is placed nowhere (its sequence numbers are the old ordering's).
    const order = this.order;
    this.video =
      m.codec === Codec.None
        ? null
        : new VideoStream(m.codec, {
            output: (frame, seq) => this.renderer.drawVideo(frame, () => (order === this.order ? order.videoOutput(seq) : null), () => {}),
            error: (error) => this.onError(error),
          });
  }

  private frame(m: MessageOf<'video'>): void {
    const verdict = this.order.video(m, performance.now());
    if (verdict.kind === 'drop' || this.video === null) return;
    this.video.decode(m.payload, verdict.keyframe, verdict.seq);
  }

  private tiles(m: MessageOf<'tiles'>): void {
    if (this.order.tiles(m) !== 'apply') return;
    const images = decodeTiles(m.rects, m.rawLen, m.data);
    m.rects.forEach((rect, i) => this.renderer.drawTile(rect, images[i]!));
  }

  destroy(): void {
    this.resizeObserver.disconnect();
    this.video?.reset();
    this.cursor.destroy();
    this.agentCursor.destroy();
    this.renderer.destroy();
    this.canvas.remove();
  }
}
