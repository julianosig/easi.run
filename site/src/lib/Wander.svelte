<script lang="ts">
  // The page's backdrop: the easi.run cursor wanders the screen behind everything and clicks now and then, each click
  // sending out two rings. Decorative. It stays away for people who ask for less motion, and rests with the tab hidden
  // (the browser stops animation frames there).
  import Logo from './Logo.svelte';

  let layer: HTMLDivElement | undefined = $state();
  let cursor: HTMLDivElement | undefined = $state();

  // Runs while the person allows motion, and stops (the cursor fading out) as soon as they ask for less.
  $effect(() => {
    if (!layer || !cursor) return;
    const root = layer;
    const el = cursor;
    const reduce = matchMedia('(prefers-reduced-motion: reduce)');
    let stop = reduce.matches ? null : wander(root, el);
    const changed = () => {
      stop?.();
      stop = reduce.matches ? null : wander(root, el);
    };
    reduce.addEventListener('change', changed);
    return () => {
      reduce.removeEventListener('change', changed);
      stop?.();
    };
  });

  /** Starts the cursor wandering; the function returned stops it and hides it. */
  function wander(root: HTMLDivElement, el: HTMLDivElement): () => void {
    let x = innerWidth * 0.18;
    let y = innerHeight * 0.4;
    let frame = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const place = () => (el.style.transform = `translate3d(${x}px, ${y}px, 0)`);
    const ease = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2);

    // It stops and clicks only where it can be seen: in the open, clear of the page's content (with room for the cursor
    // and the heart of its rings). Passing behind content on the way is fine.
    const CONTENT = 'header, iframe, h1, h2, h3, p, a, button, li, table, img';
    /** Where the page's content is on screen now, read once for every spot tried against it. */
    function content(): DOMRect[] {
      const rects: DOMRect[] = [];
      for (const node of document.querySelectorAll(CONTENT)) {
        const r = node.getBoundingClientRect();
        if (r.width > 0 && r.bottom > 0 && r.top < innerHeight) rects.push(r);
      }
      return rects;
    }
    function clear(px: number, py: number, rects: DOMRect[]): boolean {
      const left = px - 24;
      const top = py - 24;
      const right = px + 48;
      const bottom = py + 52;
      if (left < 0 || top < 0 || right > innerWidth || bottom > innerHeight) return false;
      return !rects.some((r) => r.left < right && r.right > left && r.top < bottom && r.bottom > top);
    }
    function spot(): { x: number; y: number } | null {
      const rects = content();
      for (let i = 0; i < 48; i++) {
        const p = { x: Math.random() * innerWidth, y: Math.random() * innerHeight };
        if (clear(p.x, p.y, rects)) return p;
      }
      return null;
    }

    // Off to a new spot along a gentle curve, then a click there. With no open spot on screen (a phone's narrow
    // margins), it fades out, waits and looks again.
    function move() {
      const from = { x, y };
      const to = spot();
      el.style.opacity = to ? '1' : '0';
      if (!to) {
        timer = setTimeout(move, 1500);
        return;
      }
      const duration = Math.min(2800, 800 + Math.hypot(to.x - from.x, to.y - from.y) * 2);
      const bend = (Math.random() - 0.5) * 0.6;
      const via = { x: (from.x + to.x) / 2 - (to.y - from.y) * bend, y: (from.y + to.y) / 2 + (to.x - from.x) * bend };
      const start = performance.now();
      const step = (now: number) => {
        const t = Math.min(1, (now - start) / duration);
        const e = ease(t);
        x = (1 - e) ** 2 * from.x + 2 * (1 - e) * e * via.x + e * e * to.x;
        y = (1 - e) ** 2 * from.y + 2 * (1 - e) * e * via.y + e * e * to.y;
        place();
        if (t < 1) frame = requestAnimationFrame(step);
        else timer = setTimeout(click, 200 + Math.random() * 300);
      };
      frame = requestAnimationFrame(step);
    }

    function click() {
      // The page may have scrolled content over the spot meanwhile: no click there, on to another.
      if (!clear(x, y, content())) return move();
      el.classList.add('pressed');
      setTimeout(() => el.classList.remove('pressed'), 150);
      const wave = document.createElement('span');
      wave.className = 'wave';
      wave.style.left = `${x}px`;
      wave.style.top = `${y}px`;
      root.append(wave);
      setTimeout(() => wave.remove(), 1800);
      timer = setTimeout(move, 900 + Math.random() * 1400);
    }

    const first = spot();
    if (first) ({ x, y } = first);
    place();
    el.style.opacity = first ? '1' : '0';
    timer = setTimeout(move, 700);
    return () => {
      cancelAnimationFrame(frame);
      clearTimeout(timer);
      el.style.opacity = '0';
    };
  }
</script>

<div bind:this={layer} class="pointer-events-none fixed inset-0 -z-10 overflow-hidden" aria-hidden="true">
  <div bind:this={cursor} class="cursor absolute left-0 top-0 opacity-0">
    <div class="press"><Logo class="-ml-px -mt-px h-[30px] w-auto" /></div>
  </div>
</div>

<style>
  .cursor {
    will-change: transform;
    transition: opacity 0.6s;
  }
  .press {
    transform-origin: 0 0;
    transition: transform 0.12s ease-out;
  }
  :global(.pressed) .press {
    transform: scale(0.82);
  }
  /* A click's two rings, blue then coral, centred on the cursor's point. */
  :global(.wave) {
    position: absolute;
    width: 0;
    height: 0;
  }
  :global(.wave)::before,
  :global(.wave)::after {
    content: '';
    position: absolute;
    left: -80px;
    top: -80px;
    width: 160px;
    height: 160px;
    border-radius: 9999px;
    border: 2px solid var(--color-accent);
    opacity: 0;
    animation: easi-wave 1.4s cubic-bezier(0.16, 1, 0.3, 1) forwards;
  }
  :global(.wave)::after {
    border-color: var(--color-you);
    animation-delay: 0.16s;
  }
  @keyframes -global-easi-wave {
    0% {
      transform: scale(0.05);
      opacity: 0.65;
    }
    100% {
      transform: scale(1);
      opacity: 0;
    }
  }
</style>
