// The automation agent's cursor, over the screen: where the admin says the agent acted (`agent-pointer`), it glides
// there, gives a small press and stays, so the session always shows where its agent is. It is drawn here alone, apart
// from the pointer (renderer.ts's `Cursor`): no picture the display composes, records or gives the agent holds it. It
// looks as the landing's agent does (ui/BrowserScene.svelte): the blue arrow with its label. For people who ask for
// less motion it moves at once and gives no press.

/** The arrow, as the landing draws it, in its viewBox `10 4 48 56`: its tip at (12, 6). */
const ARROW = 'M12 6 56 30 35 36 25 58Z';
/** The brand blue (app.css `--color-accent`). */
const ACCENT = '#2457f5';
/** The arrow's height, in CSS pixels, and where its tip is in it. */
const HEIGHT = 28;
const TIP = { x: (2 / 56) * HEIGHT, y: (2 / 56) * HEIGHT };
const GLIDE_MS = 350;
const PRESS_MS = 450;
/** How long it takes to fade in, the first time. */
const FADE_MS = 400;

/** Where an output point is over the screen, in CSS pixels from the container's top left; null before it has a size. */
export function agentPlace(
  x: number,
  y: number,
  screen: { offsetLeft: number; offsetTop: number; clientWidth: number; width: number },
): { left: number; top: number } | null {
  if (screen.width === 0) return null;
  const scale = screen.clientWidth / screen.width;
  return { left: screen.offsetLeft + x * scale, top: screen.offsetTop + y * scale };
}

export class AgentCursor {
  private readonly root = document.createElement('div');
  private readonly press = document.createElement('span');
  private at: { x: number; y: number } | null = null;
  private shown = false;
  /** Whether the screen has the session's size yet: before its first `config` the canvas has none of its own. */
  private sized = false;

  constructor(
    private readonly screen: HTMLCanvasElement,
    label: string,
  ) {
    this.root.setAttribute('aria-hidden', 'true');
    this.root.dataset.agentCursor = '';
    this.root.style.cssText = `position:absolute;left:0;top:0;pointer-events:none;display:flex;align-items:flex-start;opacity:0;transition:opacity ${FADE_MS}ms ease`;
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.setAttribute('viewBox', '10 4 48 56');
    svg.style.cssText = `height:${HEIGHT}px;width:auto;overflow:visible;filter:drop-shadow(0 2px 3px rgb(20 40 75 / 0.3))`;
    const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    path.setAttribute('d', ARROW);
    path.setAttribute('fill', ACCENT);
    path.setAttribute('stroke', '#ffffff');
    path.setAttribute('stroke-width', '4');
    path.setAttribute('stroke-linejoin', 'round');
    svg.append(path);
    const chip = document.createElement('span');
    chip.textContent = label;
    chip.style.cssText = `margin:${HEIGHT * 0.78}px 0 0 2px;border-radius:9999px;background:${ACCENT};color:#ffffff;padding:2px 9px;font:600 12px/1.4 system-ui,sans-serif;white-space:nowrap`;
    // A ring around the tip that grows and fades: the press.
    this.press.style.cssText = `position:absolute;left:${TIP.x - 12}px;top:${TIP.y - 12}px;width:24px;height:24px;border-radius:50%;border:2px solid ${ACCENT};box-sizing:border-box;opacity:0`;
    this.root.append(this.press, svg, chip);
    screen.insertAdjacentElement('afterend', this.root);
  }

  /** The agent acted at (x, y), in output pixels. */
  show(x: number, y: number): void {
    const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
    // The first time it appears where it is, rather than gliding in from the corner.
    const glide = this.shown && !still;
    this.root.style.transition = `opacity ${FADE_MS}ms ease${glide ? `, transform ${GLIDE_MS}ms cubic-bezier(0.65, 0, 0.35, 1)` : ''}`;
    this.at = { x, y };
    this.place();
    if (!still) {
      this.press.animate(
        [
          { transform: 'scale(0.4)', opacity: 0.7 },
          { transform: 'scale(1.6)', opacity: 0 },
        ],
        { duration: PRESS_MS, delay: glide ? GLIDE_MS : 0, easing: 'ease-out' },
      );
    }
  }

  /** The screen took the session's size (a `config`): where the agent last acted can be placed now. */
  configured(): void {
    this.sized = true;
    this.place();
  }

  /** Places it over the screen, also after the screen was resized on the page; it shows once it has a place there. */
  place(): void {
    if (this.at === null || !this.sized) return;
    const place = agentPlace(this.at.x, this.at.y, this.screen);
    if (place === null) return;
    this.root.style.transform = `translate(${place.left - TIP.x}px, ${place.top - TIP.y}px)`;
    this.shown = true;
    this.root.style.opacity = '1';
  }

  destroy(): void {
    this.root.remove();
  }
}
