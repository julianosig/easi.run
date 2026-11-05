<script lang="ts">
  // A session's chat (admin/src/chat.rs): its program's own terminal over the screen, at the bottom or at the side, as this
  // browser last left it (chatPanel.ts). Hiding it leaves the program running, and only that program exiting or the session stopping ends it; the screen stays under it. What the terminal shows comes from the program, and may
  // quote a page: its links open only as web pages, and it may write the clipboard, never read it.
  import { isDark, watchTheme } from '../../../i18n/theme.ts';
  import { t } from '../i18n/index.svelte.ts';
  import Button from './Button.svelte';
  import { clampShare, clampTransparency, loadPanel, MAX_SHARE, MAX_TRANSPARENCY, savePanel, SIDE_FROM_WIDTH } from './chatPanel.ts';
  import Icon from './Icon.svelte';
  import { ICONS } from './icons.ts';
  import ProductLogo from './ProductLogo.svelte';
  import type { ChatInfo } from './api.ts';

  let {
    id,
    chat,
    shown,
    coversBottom = $bindable(0),
    coversSide = $bindable(0),
    onHide,
    onEnded,
  }: { id: string; chat: ChatInfo; shown: boolean; coversBottom?: number; coversSide?: number; onHide: () => void; onEnded: () => void } = $props();

  /** The terminal on the page's own paper: ink, its cursor coral, the person's, its selection the brand's. Each colour reads
   *  on white, and the terminal keeps whatever a program asks for readable (`minimumContrastRatio`). */
  const TERMINAL = {
    // Clear: the panel behind it is its paper, and how much of the screen shows through.
    background: '#ffffff00',
    foreground: '#14284b',
    cursor: '#f06445',
    cursorAccent: '#ffffff',
    selectionBackground: 'rgba(36, 87, 245, 0.2)',
    selectionForeground: '#14284b',
    black: '#14284b',
    red: '#b91c1c',
    green: '#15803d',
    yellow: '#b45309',
    blue: '#2457f5',
    magenta: '#7c3aed',
    cyan: '#0e7490',
    white: '#44546e',
    brightBlack: '#5b6b85',
    brightRed: '#dc2626',
    brightGreen: '#16a34a',
    brightYellow: '#d97706',
    brightBlue: '#1a45d6',
    brightMagenta: '#9333ea',
    brightCyan: '#0891b2',
    brightWhite: '#5b6b85',
  };

  /** The same on the dark paper. */
  const TERMINAL_DARK = {
    // Clear as well, its colour what programs' text is kept readable against.
    background: '#151e2e00',
    foreground: '#e8edf5',
    cursor: '#f06445',
    cursorAccent: '#ffffff',
    selectionBackground: 'rgba(79, 129, 255, 0.35)',
    selectionForeground: '#e8edf5',
    black: '#2c3a52',
    red: '#ff7b72',
    green: '#3fb950',
    yellow: '#d29922',
    blue: '#79a6ff',
    magenta: '#c297ff',
    cyan: '#39c5cf',
    white: '#c9d4e3',
    brightBlack: '#8593a8',
    brightRed: '#ffa198',
    brightGreen: '#56d364',
    brightYellow: '#e3b341',
    brightBlue: '#a5c2ff',
    brightMagenta: '#d2a8ff',
    brightCyan: '#56d4dd',
    brightWhite: '#f0f4fa',
  };
  const palette = () => (isDark() ? TERMINAL_DARK : TERMINAL);

  let panel = $state(loadPanel());
  let windowWidth = $state(window.innerWidth);
  let windowHeight = $state(window.innerHeight);
  // Too narrow for the side, it sits at the bottom, and goes back when there is room.
  const roomy = $derived(windowWidth >= SIDE_FROM_WIDTH);
  const view = $derived(roomy ? panel.view : 'bottom');
  const length = $derived(view === 'side' ? windowWidth : windowHeight);
  const share = $derived(clampShare(panel.shares[view], view, length));
  const place = $derived(
    view === 'side'
      ? `inset-y-0 right-0 rounded-l-2xl border-l shadow-[-24px_0_60px_-24px_rgb(20_40_75/0.45)] ${shown ? '' : 'invisible translate-x-full'}`
      : `inset-x-0 bottom-0 rounded-t-2xl border-t shadow-[0_-24px_60px_-24px_rgb(20_40_75/0.45)] ${shown ? '' : 'invisible translate-y-full'}`,
  );

  $effect(() => {
    coversBottom = view === 'bottom' ? share * windowHeight : 0;
    coversSide = view === 'side' ? share * windowWidth : 0;
  });

  const resize = (to: number) => {
    panel.shares[view] = clampShare(to, view, length);
  };
  const dragged = (e: PointerEvent) => {
    const handle = e.currentTarget as HTMLElement;
    if (!handle.hasPointerCapture(e.pointerId)) return;
    resize(view === 'side' ? (windowWidth - e.clientX) / windowWidth : (windowHeight - e.clientY) / windowHeight);
  };
  const stepped = (e: KeyboardEvent) => {
    const grows = view === 'side' ? 'ArrowLeft' : 'ArrowUp';
    const shrinks = view === 'side' ? 'ArrowRight' : 'ArrowDown';
    if (e.key !== grows && e.key !== shrinks) return;
    e.preventDefault();
    resize(share + (e.key === grows ? 0.02 : -0.02));
    savePanel(panel);
  };

  /** How the admin ends a terminal (admin/src/chat.rs). */
  const CLOSE_ENDED = 4000;
  const CLOSE_REPLACED = 4001;

  let host: HTMLDivElement | undefined = $state();
  let status: 'opening' | 'open' | 'replaced' | 'lost' = $state('opening');
  let connect = () => {};
  let fit = () => {};
  let focus = () => {};

  const web = (uri: string) => {
    try {
      const { protocol } = new URL(uri);
      if (protocol === 'https:' || protocol === 'http:') window.open(uri, '_blank', 'noopener');
    } catch {
      // Not an address: nothing to open.
    }
  };

  $effect(() => {
    const element = host;
    if (!element) return;
    let gone = false;
    let socket: WebSocket | null = null;
    let cleanup = () => {};
    void (async () => {
      const [{ Terminal }, { FitAddon }, { WebLinksAddon }, { ClipboardAddon }, { WebglAddon }] = await Promise.all([
        import('@xterm/xterm'),
        import('@xterm/addon-fit'),
        import('@xterm/addon-web-links'),
        import('@xterm/addon-clipboard'),
        import('@xterm/addon-webgl'),
      ]);
      // Measured once, as the terminal opens: the font must be there by then.
      await document.fonts.load("13px 'Geist Mono Variable'").catch(() => {});
      if (gone) return;
      const terminal = new Terminal({
        allowProposedApi: true,
        allowTransparency: true,
        cursorBlink: true,
        fontFamily: "'Geist Mono Variable', monospace",
        fontSize: 13,
        minimumContrastRatio: 4.5,
        scrollback: 0,
        linkHandler: { activate: (_, uri) => web(uri) },
        theme: palette(),
      });
      const sizing = new FitAddon();
      terminal.loadAddon(sizing);
      terminal.loadAddon(new WebLinksAddon((_, uri) => web(uri)));
      terminal.loadAddon(
        new ClipboardAddon(undefined, {
          readText: async () => '',
          writeText: async (_, text) => navigator.clipboard.writeText(text),
        }),
      );
      terminal.open(element);
      // The page's style rules allow no <style> element, which the terminal's other renderer writes: without WebGL it
      // still shows its text, plainer.
      try {
        const webgl = new WebglAddon();
        webgl.onContextLoss(() => webgl.dispose());
        terminal.loadAddon(webgl);
      } catch {
        // No WebGL here.
      }
      const sendSize = () => {
        if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ cols: terminal.cols, rows: terminal.rows }));
      };
      fit = () => {
        if (element.clientWidth > 0 && element.clientHeight > 0) sizing.fit();
      };
      focus = () => terminal.focus();
      terminal.onResize(sendSize);
      const encoder = new TextEncoder();
      const typed = terminal.onData((data) => {
        if (socket?.readyState === WebSocket.OPEN) socket.send(encoder.encode(data));
      });
      const raw = terminal.onBinary((data) => {
        if (socket?.readyState === WebSocket.OPEN) socket.send(Uint8Array.from(data, (c) => c.charCodeAt(0)));
      });
      connect = () => {
        status = 'opening';
        terminal.reset();
        const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
        const opened = new WebSocket(`${scheme}://${location.host}/ws/sessions/${id}/chat`);
        opened.binaryType = 'arraybuffer';
        socket = opened;
        opened.onopen = () => {
          status = 'open';
          fit();
          sendSize();
          if (shown) terminal.focus();
        };
        opened.onmessage = (e) => {
          if (e.data instanceof ArrayBuffer) terminal.write(new Uint8Array(e.data));
        };
        opened.onclose = (e) => {
          if (gone || socket !== opened) return;
          if (e.code === CLOSE_ENDED) onEnded();
          else status = e.code === CLOSE_REPLACED ? 'replaced' : 'lost';
        };
      };
      const looks = watchTheme(() => (terminal.options.theme = palette()));
      const observer = new ResizeObserver(() => fit());
      observer.observe(element);
      fit();
      connect();
      cleanup = () => {
        looks();
        observer.disconnect();
        typed.dispose();
        raw.dispose();
        terminal.dispose();
      };
    })();
    return () => {
      gone = true;
      socket?.close();
      cleanup();
    };
  });

  $effect(() => {
    if (shown) {
      fit();
      focus();
    }
  });
</script>

<svelte:window bind:innerWidth={windowWidth} bind:innerHeight={windowHeight} />

<section
  class="fixed z-10 flex flex-col border-line transition-[translate,visibility] duration-200 ease-out motion-reduce:transition-none {place}"
  style:height={view === 'bottom' ? `${share * 100}dvh` : undefined}
  style:width={view === 'side' ? `${share * 100}vw` : undefined}
  style:background-color="color-mix(in srgb, var(--color-paper) {100 - panel.transparency}%, transparent)"
  aria-label={t('chat.label')}
  aria-hidden={!shown}
>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions: a separator that moves is a control. -->
  <div
    class="group absolute z-10 flex touch-none items-center justify-center outline-none {view === 'side'
      ? 'inset-y-0 -left-1.5 w-3 cursor-col-resize'
      : 'inset-x-0 -top-1.5 h-3 cursor-row-resize'}"
    role="separator"
    tabindex="0"
    aria-orientation={view === 'side' ? 'vertical' : 'horizontal'}
    aria-label={t('chat.resize')}
    aria-valuemin={0}
    aria-valuemax={MAX_SHARE * 100}
    aria-valuenow={Math.round(share * 100)}
    onpointerdown={(e) => {
      e.preventDefault();
      e.currentTarget.setPointerCapture(e.pointerId);
    }}
    onpointermove={dragged}
    onpointerup={() => savePanel(panel)}
    onkeydown={stepped}
  >
    <span
      class="rounded-full bg-line transition-colors group-hover:bg-accent group-focus-visible:bg-accent group-active:bg-accent {view === 'side'
        ? 'h-10 w-1'
        : 'h-1 w-10'}"
    ></span>
  </div>
  <header class="flex h-12 shrink-0 items-center gap-2.5 pl-4 pr-2">
    <ProductLogo product={chat.product} />
    <span class="flex min-w-0 flex-1 items-baseline gap-2">
      <!-- The account's name gives way first, then the product's: a narrow window keeps every control. -->
      <span class="truncate text-sm font-semibold">{t(`chat.product.${chat.product}`)}</span>
      <span class="shrink-[99999] truncate text-[13px] text-muted">{chat.name}</span>
    </span>
    <label class="flex shrink-0 items-center gap-1.5 text-muted" title={t('chat.transparency')}>
      <Icon d={ICONS.transparency} class="size-4" />
      <input
        type="range"
        class="w-20 cursor-pointer accent-accent"
        min="0"
        max={MAX_TRANSPARENCY}
        step="5"
        aria-label={t('chat.transparency')}
        value={panel.transparency}
        oninput={(e) => (panel.transparency = clampTransparency(e.currentTarget.valueAsNumber))}
        onchange={() => savePanel(panel)}
      />
      <span class="w-8 text-right text-xs tabular-nums">{panel.transparency}%</span>
    </label>
    {#if roomy}
      <Button
        variant="ghost"
        size="sm"
        icon={view === 'side' ? ICONS.dockBottom : ICONS.dockSide}
        label={t(view === 'side' ? 'chat.view.bottom' : 'chat.view.side')}
        onclick={() => {
          panel.view = view === 'side' ? 'bottom' : 'side';
          savePanel(panel);
          focus();
        }}
      />
    {/if}
    <Button variant="ghost" size="sm" icon={ICONS.collapse} label={t('chat.hide')} onclick={onHide} />
  </header>
  <div class="relative mx-2 mb-2 min-h-0 flex-1 overflow-hidden rounded-xl p-3 ring-1 ring-line">
    <div bind:this={host} class="size-full"></div>
    {#if status === 'opening'}
      <p class="absolute inset-0 grid place-items-center text-sm text-muted" role="status">{t('chat.opening')}</p>
    {:else if status === 'replaced' || status === 'lost'}
      <div class="absolute inset-0 z-10 grid place-items-center bg-paper/90 p-4">
        <div class="text-center text-sm" role="status">
          <p>{t(status === 'replaced' ? 'chat.replaced' : 'chat.lost')}</p>
          <div class="mt-3 flex justify-center">
            <Button size="sm" onclick={() => connect()}>{t(status === 'replaced' ? 'chat.here' : 'chat.reconnect')}</Button>
          </div>
        </div>
      </div>
    {/if}
  </div>
</section>
