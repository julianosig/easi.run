<script lang="ts">
  // The film: every feature of the Features page in its order, as one headline over its drawing, the camera whipping
  // from one to the next; opened by the mark and closed by the install prompt. Its time is `time` seconds: it plays on
  // its own, and scripts/film.mjs seeks it frame by frame (`window.film`), each drawing's animations following.
  import { tick } from 'svelte';
  import { type Card, FIRST, GROUPS } from '../lib/features/index.ts';
  import Wordmark from '../lib/Wordmark.svelte';
  import { NAME, ORIGIN } from '../lib/site.ts';
  import { catalog, has, locale, say, t } from './i18n.ts';

  // Every word on screen is film.* (messages/en/film.json): a card's headline film.card.<card>, a group's
  // film.group.<group>; `*word*` is drawn in color.

  /** Each group's ground. */
  const TONES: Record<string, string> = { agents: 'bg-accent', you: 'bg-you', security: 'bg-void', server: 'bg-accent' };

  type Shown = { length: number } & (
    | { kind: 'card'; card: Card; number: number }
    | { kind: 'group'; group: string; count: number }
    | { kind: 'outro' }
  );
  type Slide = Shown & { start: number };

  /** The opening's lines (film.opening.<n>.1 and .2), cut one to the next, each on its ground and moving its way in. */
  type Motion = 'slam' | 'rise' | 'slide';
  const OPENING: { from: number; to: number; motion: Motion; ground: string; mark: string }[] = [
    { from: 0, to: 2, motion: 'slam', ground: 'light', mark: 'text-accent' },
    { from: 2, to: 3.7, motion: 'rise', ground: 'bg-void text-white', mark: 'text-[#8aa8ff]' },
    { from: 3.7, to: 5.4, motion: 'slide', ground: 'bg-accent text-white', mark: 'text-[#ffd3c9]' },
    { from: 5.4, to: 7, motion: 'slam', ground: 'light', mark: 'text-accent' },
    { from: 7, to: 8.6, motion: 'rise', ground: 'bg-void text-white', mark: 'text-[#8aa8ff]' },
  ];
  /** Then the mark, whose coral dot grows over the screen and opens on the first card. */
  const MARK = 8.6;
  const INTRO = MARK + 2.8;
  const slides: Slide[] = [];
  let clock = INTRO + 0.15;
  let number = 0;
  const add = (slide: Shown) => {
    slides.push({ ...slide, start: clock });
    clock += slide.length;
  };
  for (const card of FIRST) add({ kind: 'card', card, number: ++number, length: 3.2 });
  for (const group of GROUPS) {
    add({ kind: 'group', group: group.id, count: group.cards.length, length: 1.8 });
    for (const card of group.cards) add({ kind: 'card', card, number: ++number, length: 2.6 });
  }
  add({ kind: 'outro', length: 7.5 });
  const DURATION = clock;

  const clamp = (x: number) => Math.min(1, Math.max(0, x));
  const out3 = (x: number) => 1 - (1 - clamp(x)) ** 3;
  const outBack = (x: number) => {
    const k = clamp(x) - 1;
    return 1 + 2.7 * k ** 3 + 1.7 * k ** 2;
  };
  const inOutExpo = (x: number) => {
    x = clamp(x);
    if (x === 0 || x === 1) return x;
    return x < 0.5 ? 2 ** (20 * x - 10) / 2 : (2 - 2 ** (-20 * x + 10)) / 2;
  };
  const inExpo = (x: number) => (clamp(x) === 0 ? 0 : 2 ** (10 * clamp(x) - 10));

  /** The camera: on slide `i`, whipping to the next in the `PAN` s around its start. */
  const PAN = 0.7;
  function camera(at: number) {
    let position = 0;
    for (let i = 1; i < slides.length; i++) {
      const s = slides[i].start;
      if (at >= s + PAN / 2) position = i;
      else if (at > s - PAN / 2) return i - 1 + inOutExpo((at - (s - PAN / 2)) / PAN);
      else break;
    }
    return position;
  }

  let time = $state(0);
  const cam = $derived(camera(time));
  /** The whip's speed, in slides a second, for its motion blur and the camera's pull back. */
  const speed = $derived(Math.abs(camera(time + 1 / 60) - camera(time - 1 / 60)) * 30);
  const pull = $derived(Math.sin(Math.PI * (cam - Math.floor(cam))));
  const visible = $derived(slides.map((_, i) => i).filter((i) => Math.abs(i - cam) < 1.05));

  /** The headline's words, each with its emphasis and whether a space follows it; Chinese and Japanese, written
   *  without spaces, in the words a segmenter finds. */
  type Word = { text: string; strong: boolean; space: boolean };
  const segmenter = new Intl.Segmenter(locale(), { granularity: 'word' });
  const spaceless = ['zh-CN', 'ja'].includes(locale());
  /** The font for the scripts Plus Jakarta Sans lacks, by language (video/index.html loads them). */
  const SCRIPT: Partial<Record<string, string>> = { ja: 'Noto Sans JP', 'zh-CN': 'Noto Sans SC', ko: 'Noto Sans KR', hi: 'Noto Sans Devanagari' };
  const script = SCRIPT[locale()];
  function words(line: string): Word[] {
    const found: Word[] = [];
    for (const run of line.split(/(\*[^*]+\*)/)) {
      const strong = /^\*[^*]+\*$/.test(run);
      const text = strong ? run.slice(1, -1) : run;
      const pieces = spaceless ? [...segmenter.segment(text)].map((s) => s.segment) : text.split(/(\s+)/);
      for (const piece of pieces) {
        if (!piece) continue;
        if (/^\s+$/.test(piece)) {
          if (found.length) found[found.length - 1].space = true;
        } else found.push({ text: piece, strong, space: false });
      }
    }
    return found;
  }

  /** The size, at most `largest` px, at which `text` takes no more than `room` px of line: a wide character (Chinese,
   *  Japanese, Korean) counted an em, any other half of one. */
  const fit = (text: string, largest: number, room: number) =>
    Math.min(largest, room / [...text.replaceAll('*', '')].reduce((em, c) => em + (c.codePointAt(0)! >= 0x2e80 ? 1 : 0.52), 0));

  // The opening's mark: its cursor 260 px tall, centred, 230 px from the top; the dot's centre is where the coral grows.
  const LOGO = { height: 260, top: 230 };
  const SCALE = LOGO.height / 56;
  const DOT = { x: 960 - (48 * SCALE) / 2 + (51 - 10) * SCALE, y: LOGO.top + (51 - 4) * SCALE };
  const grown = $derived(inExpo((time - (INTRO - 0.65)) / 0.65) * 2400);
  const opened = $derived(time < INTRO ? 0 : out3((time - INTRO) / 0.75) * 2400);

  const arts: HTMLElement[] = $state([]);

  /** The drawings at this moment: each plays its whole loop over its slide, at most three times as fast. */
  function animate() {
    for (const i of visible) {
      const slide = slides[i];
      const node = arts[i];
      if (slide.kind !== 'card' || !node) continue;
      const animations = node.getAnimations({ subtree: true });
      const loop = Math.max(0, ...animations.map((a) => Number(a.effect?.getTiming().duration) || 0)) / 1000;
      const rate = Math.min(3, Math.max(1, loop / (slide.length + 0.5)));
      for (const animation of animations) {
        animation.pause();
        animation.currentTime = Math.max(0, time - slide.start + 0.3) * rate * 1000;
      }
    }
  }

  const film = {
    duration: DURATION,
    /** Shows the film at `at` seconds, the drawings there too. */
    async seek(at: number) {
      time = Math.min(DURATION, Math.max(0, at));
      await tick();
      animate();
    },
    /** Waits for the fonts: the language's script font in every piece the film's words need, before any frame. */
    async ready() {
      if (script) {
        const words = Object.values(catalog()).flatMap((m) => (typeof m === 'string' ? [m] : Object.values(m))).join('');
        await Promise.all(['500', '600'].map((weight) => document.fonts.load(`${weight} 100px "${script}"`, words)));
      }
      await document.fonts.ready;
    },
  };
  (window as unknown as { film: typeof film }).film = film;

  // Played in a browser (not captured), it runs on its own, in a loop.
  const capture = new URLSearchParams(location.search).has('capture');
  $effect(() => {
    if (capture) return;
    let frame = 0;
    const began = performance.now();
    const step = () => {
      void film.seek(((performance.now() - began) / 1000) % DURATION);
      frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  });

  const prompt = `Read ${ORIGIN}/install.md and install ${NAME} on this server.`;
  const TYPED_AT = 1 + prompt.length / 42;
</script>

<!-- A word of the opening coming in its way, `local` s into its line: slammed from big and blurred, risen out of its
     line, or slid in from the right. -->
{#snippet entering(text: string, mark: string, motion: Motion, local: number)}
  {#if motion === 'rise'}
    {@const p = out3(local / 0.5)}
    <span class="inline-block overflow-hidden pb-[0.08em] align-bottom"
      ><span class="inline-block {mark}" style:transform="translateY({(1 - p) * 110}%) rotate({(1 - p) * 6}deg)">{text}</span></span
    >
  {:else if motion === 'slide'}
    {@const p = out3(local / 0.45)}
    <span
      class="inline-block {mark}"
      style:opacity={p}
      style:filter="blur({(1 - p) * 10}px)"
      style:transform="translateX({(1 - p) * 320}px) skewX({(1 - p) * -14}deg)">{text}</span
    >
  {:else}
    {@const p = out3(local / 0.35)}
    <span
      class="inline-block {mark}"
      style:opacity={p}
      style:filter="blur({(1 - p) * 14}px)"
      style:transform="scale({1.4 - 0.4 * p})">{text}</span
    >
  {/if}
{/snippet}

<!-- A headline rising word by word out of its line, `local` s into its slide; `mark` the emphasis's class. -->
{#snippet headline(line: string, local: number, mark: string, underline: boolean)}
  {#each words(line) as word, j (j)}
    {@const p = out3((local + 0.15 - j * Math.min(0.06, 0.5 / words(line).length)) / 0.5)}
    <span class="relative inline-block overflow-hidden align-bottom {underline ? 'pb-[0.24em]' : 'pb-[0.08em]'}"
      ><span class="inline-block {word.strong ? mark : ''}" style:transform="translateY({(1 - p) * 110}%) rotate({(1 - p) * 6}deg)"
        >{word.text}</span
      >{#if underline && word.strong}<span
          class="absolute inset-x-0 bottom-0 h-[0.09em] origin-left rounded-full bg-white"
          style:transform="scaleX({out3((local - 0.35 - j * Math.min(0.06, 0.5 / words(line).length)) / 0.4)})"
        ></span>{/if}</span
    >{word.space ? ' ' : ''}
  {/each}
{/snippet}

<div
  class="relative h-[1080px] w-[1920px] overflow-hidden bg-paper font-sans text-ink"
  style:--font-sans={script ? `'Plus Jakarta Sans Variable', '${script}', sans-serif` : undefined}
  style:--font-display={script ? `'Plus Jakarta Sans Variable', '${script}', sans-serif` : undefined}
>
  <!-- The opening. -->
  {#if time < INTRO + 0.8}
    {@const flown = inOutExpo((time - MARK - 0.05) / 0.6)}
    {@const pop = outBack((time - MARK - 0.55) / 0.4)}
    {@const fade = 1 - out3((time - (INTRO - 0.4)) / 0.3)}
    {@const named = out3((time - MARK - 0.75) / 0.45)}
    {@const now = OPENING.find((o) => time >= o.from && time < o.to)}
    <div class="absolute inset-0 bg-[radial-gradient(circle_at_50%_45%,#ffffff_0%,#eaf0ff_100%)]">
      {#if now}
        {@const gone = out3((time - (now.to - 0.25)) / 0.25)}
        {@const n = OPENING.indexOf(now) + 1}
        {@const lines = [say(`film.opening.${n}.1`), say(`film.opening.${n}.2`)]}
        {@const first = words(lines[0])}
        <div class="absolute inset-0 {now.ground === 'light' ? '' : now.ground}">
          <h1
            class="absolute inset-x-0 top-1/2 -translate-y-1/2 text-center font-semibold leading-[1.05] tracking-[-0.04em]"
            style:font-size="{Math.min(fit(lines[0], 150, 1720), fit(lines[1], 150, 1720))}px"
            style:opacity={1 - gone}
            style:filter="blur({gone * 12}px)"
            style:transform="scale({1 + 0.15 * gone})"
          >
              {#each [...first, ...words(lines[1])] as word, j (j)}{@render entering(
                word.text,
                word.strong ? now.mark : '',
                now.motion,
                time - now.from - 0.1 - j * Math.min(now.motion === 'slam' ? 0.12 : 0.07, 0.7 / (first.length + words(lines[1]).length)),
              )}{#if j === first.length - 1}<br />{:else if word.space}{' '}{/if}{/each}
          </h1>
        </div>
      {:else}
        <svg
          viewBox="10 4 48 56"
          class="absolute w-auto overflow-visible"
          style:height="{LOGO.height}px"
          style:left="{960 - (48 * SCALE) / 2}px"
          style:top="{LOGO.top}px"
          aria-hidden="true"
        >
          <path
            d="M12 6 56 30 35 36 25 58Z"
            class="fill-accent stroke-accent"
            stroke-width="3.5"
            stroke-linejoin="round"
            style:transform="translate({(1 - flown) * -140}px, {(1 - flown) * -90}px) rotate({(1 - flown) * -50}deg)"
            style:transform-origin="34px 32px"
            style:opacity={clamp(flown * 3) * fade}
          />
          <circle cx="51" cy="51" r="6.5" class="fill-you" style:transform="scale({pop})" style:transform-origin="51px 51px" />
        </svg>
        <div class="absolute inset-x-0 top-[540px] text-center" style:opacity={fade}>
          <div style:opacity={named} style:transform="translateY({(1 - named) * 60}px)">
            <Wordmark class="text-[170px]" />
          </div>
          <p class="mt-10 text-[54px] font-semibold tracking-[-0.02em] text-muted">
            {@render entering(t('film.mark.1'), '', 'slam', time - MARK - 1.25)}
            {@render entering(t('film.mark.2'), '', 'slam', time - MARK - 1.45)}
            {@render entering(t('film.mark.3'), 'text-accent', 'slam', time - MARK - 1.65)}
          </p>
        </div>
      {/if}
      <!-- The dot grows over everything. -->
      <div
        class="absolute rounded-full bg-you"
        style:left="{DOT.x - grown}px"
        style:top="{DOT.y - grown}px"
        style:width="{grown * 2}px"
        style:height="{grown * 2}px"
      ></div>
    </div>
  {/if}

  <!-- The cards, side by side, the camera whipping along them; opened from the dot. -->
  {#if time >= INTRO}
    <div
      class="absolute inset-0 overflow-hidden"
      style:clip-path={opened < 2400 ? `circle(${opened}px at ${DOT.x}px ${DOT.y}px)` : 'none'}
    >
      <!-- The ground drifts on its own. -->
      <div class="absolute inset-0 bg-[linear-gradient(135deg,#ffffff_0%,#f3f6fb_60%,#eaf0ff_100%)]">
        <div
          class="absolute size-[900px] rounded-full bg-accent/10 blur-[120px]"
          style:left="{1200 + Math.sin(time * 0.5) * 260 - cam * 40}px"
          style:top="{-200 + Math.cos(time * 0.4) * 160}px"
        ></div>
        <div
          class="absolute size-[700px] rounded-full bg-you/10 blur-[120px]"
          style:left="{-100 + Math.cos(time * 0.45) * 220 - cam * 25}px"
          style:top="{560 + Math.sin(time * 0.6) * 140}px"
        ></div>
      </div>

      <svg class="absolute size-0" aria-hidden="true">
        <filter id="whip" x="-10%" y="0" width="120%" height="100%">
          <feGaussianBlur stdDeviation="{Math.min(36, speed * 4.5)} 0" />
        </filter>
      </svg>
      <div
        class="absolute inset-0"
        style:filter={speed > 0.08 ? 'url(#whip)' : 'none'}
        style:transform="scale({1 - 0.08 * pull})"
      >
        {#each visible as i (i)}
          {@const slide = slides[i]}
          {@const local = time - slide.start}
          {@const offset = i - cam}
          {@const settle = 1 + 0.035 * clamp(local / slide.length)}
          <div class="absolute inset-0" style:transform="translateX({offset * 1920}px)">
            {#if slide.kind === 'card'}
              {@const Art = slide.card.art}
              {@const arrive = outBack((local + 0.3) / 0.7)}
              <p
                class="absolute bottom-[-150px] right-[-30px] font-display text-[620px] font-semibold leading-none tracking-[-0.06em] text-transparent [-webkit-text-stroke:4px_rgb(36_87_245/0.12)]"
                style:transform="translateX({offset * -700}px)"
              >
                {String(slide.number).padStart(2, '0')}
              </p>
              <h2
                class="absolute inset-x-[100px] top-[110px] text-center font-semibold leading-[1.05] tracking-[-0.04em]"
                style:font-size="{fit(say(`film.card.${slide.card.id}`), 98, 3000)}px"
                style:transform="translateX({offset * 300}px) scale({settle})"
              >
                {@render headline(say(`film.card.${slide.card.id}`), local, 'text-accent', false)}
              </h2>
              <div class="absolute inset-x-0 top-[400px] flex justify-center [perspective:2400px]">
                <div
                  bind:this={arts[i]}
                  class="w-[1240px] rounded-[44px] bg-white p-12 shadow-[0_60px_140px_-50px_rgb(20_40_75/0.5)] ring-1 ring-line"
                  style:opacity={clamp((local + 0.45) / 0.3)}
                  style:transform="translateX({offset * 900}px) rotateY({offset * -28}deg) scale({(0.8 + 0.2 * arrive) * settle})"
                >
                  <!-- The drawing at the Features page's size, scaled up whole: its own layout kept. -->
                  <div class="w-[344px] [zoom:3.3]"><Art /></div>
                </div>
              </div>
              {#if has(`film.note.${slide.card.id}`)}
                <p
                  class="absolute inset-x-0 bottom-[44px] text-center text-[26px] text-muted"
                  style:opacity={out3((local - 0.6) / 0.4)}
                >
                  {say(`film.note.${slide.card.id}`)}
                </p>
              {/if}
            {:else if slide.kind === 'group'}
              {@const counted = Math.round(slide.count * out3((local - 0.1) / 0.8))}
              <div class="absolute inset-0 {TONES[slide.group]} text-white">
                <div
                  class="absolute size-[1300px] rounded-full border-[3px] border-white/15"
                  style:left="{1100 + offset * -500}px"
                  style:top="{-300 + local * 40}px"
                  style:transform="scale({0.8 + 0.25 * out3(local / slide.length)})"
                ></div>
                <div
                  class="absolute size-[700px] rounded-full bg-white/10"
                  style:left="{-200 + offset * -900}px"
                  style:top="{620 - local * 60}px"
                ></div>
                <div class="absolute inset-x-[160px] top-[330px]" style:transform="translateX({offset * 400}px)">
                  <p class="font-display text-[64px] font-semibold tabular-nums text-white/75">{t('film.group.count', { count: counted })}</p>
                  <h2
                    class="mt-4 font-semibold leading-[1] tracking-[-0.05em]"
                    style:font-size="{fit(say(`film.group.${slide.group}`), 190, 2900)}px"
                  >
                    {@render headline(say(`film.group.${slide.group}`), local, '', true)}
                  </h2>
                </div>
              </div>
            {:else}
              {@const typed = Math.floor(Math.max(0, local - 1) * 42)}
              {@const done = typed >= prompt.length}
              {@const mark = outBack((local - TYPED_AT - 0.3) / 0.6)}
              <div class="absolute inset-0 bg-void text-white">
                <div
                  class="absolute size-[1100px] rounded-full bg-accent/25 blur-[140px]"
                  style:left="{400 + Math.sin(local) * 200}px"
                  style:top="{-300 + Math.cos(local * 0.8) * 100}px"
                ></div>
                <h2
                  class="absolute inset-x-[120px] top-[190px] text-center font-semibold leading-[1.05] tracking-[-0.04em]"
                  style:font-size="{fit(t('film.outro'), 120, 1650)}px"
                  style:transform="translateX({offset * 300}px)"
                >
                  {@render headline(t('film.outro'), local, 'text-[#8aa8ff]', false)}
                </h2>
                <div
                  class="absolute left-1/2 top-[480px] w-[1580px] -translate-x-1/2 whitespace-nowrap rounded-[32px] bg-white/[0.07] px-14 py-11 font-mono text-[35px] leading-[1.4] ring-1 ring-white/15"
                  style:opacity={out3((local - 0.5) / 0.4)}
                >
                  <span class="text-[#8aa8ff]">&gt;</span>
                  {prompt.slice(0, typed)}<span
                    class="ml-1 inline-block h-[1.05em] w-[0.5em] translate-y-[0.18em] bg-you"
                    style:opacity={!done || Math.floor(local * 2.4) % 2 === 0 ? 1 : 0}
                  ></span>
                </div>
                <div
                  class="absolute inset-x-0 top-[760px] text-center"
                  style:opacity={clamp((local - TYPED_AT - 0.3) / 0.25)}
                  style:transform="scale({0.6 + 0.4 * mark})"
                >
                  <Wordmark class="text-[150px] text-white" mark="inverse" />
                </div>
              </div>
            {/if}
          </div>
        {/each}
      </div>
    </div>
  {/if}
</div>
