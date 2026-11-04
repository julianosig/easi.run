<script lang="ts">
  // A session's chat (admin/src/chat.rs): its program's own terminal over the lower half of the window, full width. Hiding
  // it leaves the program running, and only that program exiting or the session stopping ends it; the screen stays under it. What the terminal shows comes from the program, and may
  // quote a page: its links open only as web pages, and it may write the clipboard, never read it.
  import { t } from '../i18n/index.svelte.ts';
  import Button from './Button.svelte';
  import { ICONS } from './icons.ts';
  import ProductLogo from './ProductLogo.svelte';
  import type { ChatInfo } from './api.ts';

  let {
    id,
    chat,
    shown,
    onHide,
    onEnded,
  }: { id: string; chat: ChatInfo; shown: boolean; onHide: () => void; onEnded: () => void } = $props();

  /** The terminal on the page's own paper: ink, its cursor coral, the person's, its selection the brand's. Each colour reads
   *  on white, and the terminal keeps whatever a program asks for readable (`minimumContrastRatio`). */
  const TERMINAL = {
    background: '#ffffff',
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
        cursorBlink: true,
        fontFamily: "'Geist Mono Variable', monospace",
        fontSize: 13,
        minimumContrastRatio: 4.5,
        scrollback: 0,
        linkHandler: { activate: (_, uri) => web(uri) },
        theme: TERMINAL,
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
      const observer = new ResizeObserver(() => fit());
      observer.observe(element);
      fit();
      connect();
      cleanup = () => {
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

<section
  class="fixed inset-x-0 bottom-0 z-30 flex h-[50dvh] flex-col rounded-t-2xl border-t border-line bg-paper shadow-[0_-24px_60px_-24px_rgb(20_40_75/0.45)] transition-[translate,visibility] duration-200 ease-out motion-reduce:transition-none {shown ? '' : 'invisible translate-y-full'}"
  aria-label={t('chat.label')}
  aria-hidden={!shown}
>
  <header class="flex h-12 shrink-0 items-center gap-2.5 pl-4 pr-2">
    <ProductLogo product={chat.product} />
    <span class="flex min-w-0 flex-1 items-baseline gap-2">
      <span class="shrink-0 text-sm font-semibold">{t(`chat.product.${chat.product}`)}</span>
      <span class="truncate text-[13px] text-muted">{chat.name}</span>
    </span>
    <Button variant="ghost" size="sm" icon={ICONS.collapse} label={t('chat.hide')} onclick={onHide} />
  </header>
  <div class="relative mx-2 mb-2 min-h-0 flex-1 overflow-hidden rounded-xl bg-paper p-3 ring-1 ring-line">
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
