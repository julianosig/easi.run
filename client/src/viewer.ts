// The easi.run viewer: one WebSocket, one canvas, and (only while this connection holds the lease) input. Its media come
// over the direct path instead where UDP reaches the server (see udp.ts).

import { Sound, type SoundState } from './audio.ts';
import { Copies, type CopyOffer } from './copy.ts';
import { type Download, Downloads } from './files.ts';
import { type BatchProgress, type Upload, Uploads } from './uploads.ts';
import { probeCodecs, type Probe, VideoStream } from './decoder.ts';
import { feedsComposition, isApple, isModifier, isPasteShortcut, isShortcut, keyDown, keyText, optionAsControl, remoteKey, textChunks, toOutput, travelsAs, utf8Length, wheelPixels } from './input.ts';
import { evdevCode } from './keys.ts';
import { p50 } from './median.ts';
import { StreamOrder } from './order.ts';
import {
  Codec,
  decode,
  encode,
  isMediaType,
  KeyKind,
  LeaseHolder,
  MAX_CHOSEN,
  MAX_CLIPBOARD,
  type ClientMessage,
  type Message,
  type MessageOf,
  PointerKind,
  type CameraLayout,
  type ServerMessage,
  VERSION,
} from './protocol.ts';
import { ALL_FEATURES, type Features } from './features.ts';
import { decodeTiles } from './tile.ts';
import { microphoneSupported } from './recorder.ts';
import { createRenderer, Cursor } from './renderer.ts';
import { Handover, openSession, RETRY_MS, type Session } from './udp.ts';

/** One attempt at a direct path's session: its session once open, and whether it ended. */
interface Attempt {
  session: Session | null;
  closed: boolean;
}

/** A fresh name for one viewer connection (32 hex characters), by which the page asks for and gives back control. */
export function viewerToken(): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, '0')).join('');
}

const PING_MS = 2000;
const STATS_MS = 1000;
/** Clock samples kept; the offset comes from the one with the shortest round trip. */
const CLOCK_SAMPLES = 8;
/** Round trips the current latency is the median of. */
const RECENT_TRIPS = 3;
/** How often a stuck decoder is looked for (see `VideoStream.stuck`). */
const WATCHDOG_MS = 250;
/** Pointer moves are skipped while this much is unsent; past the hard bound the connection is closed. */
const MOVE_SEND_BUDGET = 64 * 1024;
/** Each button's bit in a pointer event's `buttons`: the main (0), the middle (1), the secondary (2). */
const BUTTON_BITS = [1, 4, 2];
const MAX_SEND_BUFFER = 1024 * 1024;
/** A recording's camera picture waits while this much is queued to send: pictures are dropped, never queued. */
const RECORD_BUFFER = 256 * 1024;
// Why this side closes the stream, as its close code (browsers only send 1000 or 3000-4999), so the far end and its logs
// tell a refused message apart from a stalled connection or a browser that cannot go on.
/** A message from the server was refused (it did not decode, or came out of order). */
const CLOSE_PROTOCOL = 4002;
/** What this side sends stopped draining. */
const CLOSE_STALLED = 4003;
/** This browser cannot go on drawing (its WebGL context was lost, the codec probe failed). */
const CLOSE_CLIENT = 4004;
/** The connection itself failed or closed: nothing was wrong with what either side said. */
const CLOSE_NORMAL = 1000;

export interface ViewerStats {
  /** Frames presented in the last interval (drawn on an animation frame). */
  presented: number;
  /** Median decode time of the interval's video frames, in microseconds. */
  decodeUsP50: number;
  /** The codec of the current configuration. */
  codec: Codec | null;
  /** Whether the browser reports a hardware decoder for that codec (null when it does not say). */
  hardware: boolean | null;
  /** How the screen is drawn: WebGL2, or Canvas 2D where WebGL2 would run on a software rasterizer. */
  renderer: 'webgl2' | 'canvas2d';
  /** Media bytes (video payloads and tile data) received in the interval. */
  mediaBytes: number;
  /** All bytes received in the interval. */
  bytes: number;
  /**
   * Median time from composition (the video message's display timestamp) to the frame being drawn, in microseconds,
   * through the clock offset below; null without video frames or a clock sample.
   */
  composeToDrawUsP50: number | null;
  /** Median time from composition to the video message's arrival, in microseconds (the server and network part). */
  composeToArriveUsP50: number | null;
  /** Server clock minus `performance.now()`, in microseconds, and its error bound (half the round trip). */
  clockOffsetUs: number | null;
  clockErrorUs: number | null;
  /** The median of the last few round trips to the server, in microseconds; null when none is recent. */
  roundTripUs: number | null;
  /** The server's latest encoder statistics. */
  server: MessageOf<'server-stats'> | null;
  /** What carries the media: the WebSocket, or the direct path (UDP). */
  transport: 'websocket' | 'udp';
}

/** The control lease, as the display announces it. */
export type Lease = Omit<MessageOf<'lease'>, 'type'>;

/** The message types each feature carries from a viewer. */
const FEATURE_OF: Partial<Record<Message['type'], keyof Features>> = {
  pointer: 'keyboard_pointer',
  wheel: 'keyboard_pointer',
  key: 'keyboard_pointer',
  clipboard: 'clipboard',
  'upload-start': 'uploads',
  'upload-data': 'uploads',
  'file-choose': 'uploads',
  'file-drop': 'uploads',
  'file-drag': 'uploads',
  'file-drag-end': 'uploads',
  'record-start': 'recording',
  'record-camera': 'recording',
  'record-microphone': 'recording',
  'record-stop': 'recording',
  'record-layout': 'recording',
};

export interface ViewerOptions {
  url: string;
  /** The session's features (see `Features`); every one on when left out. What is off is never sent or offered here. */
  features?: Features;
  /** The session's id: downloads are fetched from its download endpoint. */
  session: string;
  onStats?: (stats: ViewerStats) => void;
  onLease?: (lease: Lease) => void;
  /**
   * Text the page copied while this connection held control. `written` says it is already on the person's clipboard
   * (automatic copying is on and the browser allowed it); otherwise the application offers to copy it, on a click,
   * with `writeCopied`. `null` withdraws the offer: control changed hands, or the stream ended.
   */
  onCopied?: (copy: CopyOffer) => void;
  /** The uploads this connection sent or is sending, whenever one changes. */
  onUploads?: (uploads: Upload[]) => void;
  /**
   * A batch of files on its way to the page, from the moment it is accepted until it ends: a drop's (with where it was
   * dropped, in CSS pixels from the container's top left) or a file chooser's (`at` null).
   */
  onTransfer?: (transfer: Transfer) => void;
  /**
   * The page opened a file chooser and the person's browser would not open their file picker without a fresh click:
   * the application offers `choose` (call it from a click) and `cancel`. `null` withdraws the request.
   */
  onFileRequest?: (request: { choose: () => void; cancel: () => void } | null) => void;
  /** The downloads this connection's browser is fetching or fetched, whenever one changes. */
  onDownloads?: (downloads: Download[]) => void;
  /** A recording's state, from the display: started (and offered as `file`, which downloads as it grows), ended, or
   * failed. */
  onRecording?: (state: RecordingState) => void;
  /** Whether the browser's sound plays here; it starts unmuted. Without the `sound` feature it is `unsupported`. */
  onSound?: (state: SoundState) => void;
  /** `fatal` errors closed the stream; others (a decoder error, a refused paste) were recovered from. */
  onError?: (error: Error, fatal: boolean) => void;
  /** The screen has a picture again: the first tiles or video frame drawn after each new configuration (until then the
   * canvas is blank). */
  onPicture?: () => void;
  /** Measurement only: called after each drawn frame with a reader of its pixels (output coordinates). */
  onPresent?: (read: (x: number, y: number, w: number, h: number) => Uint8Array) => void;
}

/** Where a recording is, as the display says. */
export type RecordingState = Omit<MessageOf<'record-state'>, 'type' | 'leaseEpoch'>;

/** What a recording asks the display for. */
export type RecordingRequest = Omit<MessageOf<'record-start'>, 'type' | 'leaseEpoch' | 'recording'>;

/** One batch's progress (see `onTransfer`); `batch` tells batches apart. */
export interface Transfer extends BatchProgress {
  batch: number;
  at: { x: number; y: number } | null;
}

export interface Viewer {
  /** Asks to send input; input flows only while the server also says this connection holds the lease. */
  setControl(enabled: boolean): void;
  /** Writes the offered copy to the person's clipboard (call it from their click); refused once it was withdrawn. */
  writeCopied(): Promise<void>;
  /** Turns automatic copying of what the page copies onto the person's clipboard on or off (off at first). */
  setAutoCopy(enabled: boolean): void;
  /** Mutes or unmutes the browser's sound, here only. */
  setMuted(muted: boolean): void;
  /** Sets the sound's volume here, from 0 to 1. */
  setVolume(volume: number): void;
  /**
   * Asks the display to record (the lease holder only, view only included): its number, or null without the lease or
   * the connection, or when asking ended the connection. The display answers through `onRecording`.
   */
  startRecording(request: RecordingRequest): number | null;
  /** One camera picture of recording `id`, taken at `takenMs` (`performance.now`); false when not sent (no clock yet,
   * or the connection is busy: a picture is dropped rather than queued). */
  recordCamera(id: number, jpeg: Uint8Array, takenMs: number): boolean;
  /** One Opus packet of the microphone for recording `id`, starting at `startedMs` (`performance.now`). */
  recordMicrophone(id: number, packet: Uint8Array, startedMs: number): void;
  /** The camera of recording `id` moved or changed its look. */
  recordLayout(id: number, layout: CameraLayout): void;
  stopRecording(id: number): void;
  close(): void;
}

const nowUs = (): number => performance.now() * 1000;

const evdevCodes = (keys: string[]): ReadonlySet<number> =>
  new Set(keys.map((key) => evdevCode(key)).filter((code) => code !== undefined));
/** The modifiers a Mac's Option holds (Alt, both sides), as evdev codes. */
const OPTION_KEYS = evdevCodes(['AltLeft', 'AltRight']);
/** Control and Shift, both sides, as evdev codes. */
const CONTROL_KEYS = evdevCodes(['ControlLeft', 'ControlRight']);
const SHIFT_KEYS = evdevCodes(['ShiftLeft', 'ShiftRight']);
/** The Control a Mac's Option travels as with an arrow, Backspace, or Delete (see optionAsControl). */
const OPTION_CONTROL = evdevCode('ControlLeft') as number;
/** The key each editing intent of a virtual keyboard stands for, when it comes without a key of its own. */
const EDIT_KEYS: Partial<Record<string, number>> = {
  deleteContentBackward: evdevCode('Backspace'),
  deleteContentForward: evdevCode('Delete'),
  insertLineBreak: evdevCode('Enter'),
  insertParagraph: evdevCode('Enter'),
};

export function mountViewer(container: HTMLElement, options: ViewerOptions): Viewer {
  const canvas = document.createElement('canvas');
  canvas.style.cssText = 'display:block;max-width:100%;height:auto;touch-action:none';
  // The screen itself, as a page styles it apart from the pointer's overlay.
  canvas.dataset.screen = '';
  // Keyboard and IME input land in a transparent text area over the screen, so composition works.
  const keyboard = document.createElement('textarea');
  keyboard.setAttribute('aria-label', 'Remote keyboard');
  keyboard.autocapitalize = 'off';
  keyboard.spellcheck = false;
  keyboard.style.cssText =
    'position:absolute;left:0;top:0;width:1px;height:1px;opacity:0;padding:0;border:0;resize:none;pointer-events:none';
  if (getComputedStyle(container).position === 'static') container.style.position = 'relative';
  container.append(canvas, keyboard);

  // `ended`: the stream is over (closed by either side or refused); `unmounted`: the viewer was removed.
  let ended = false;
  let unmounted = false;
  let socket: WebSocket | null = null;
  // The direct path: the attempt at an offer's session (its session once open), where the media come from, and the
  // next time it is asked for.
  let attempt: Attempt | null = null;
  const handover = new Handover();
  let retry: ReturnType<typeof setTimeout> | null = null;
  let probe: Probe = { codecs: 0, hardware: {} };
  const order = new StreamOrder();
  let video: VideoStream | null = null;
  let lease: Lease | null = null;
  let control = false;
  let autoCopy = false;
  /** The page's open file chooser, until it is answered or closed; `cancel` answers it with nothing chosen. */
  let openRequest: { request: number; close: () => void; cancel: () => void } | null = null;
  let lastServer: MessageOf<'server-stats'> | null = null;
  let bytes = 0;
  let mediaBytes = 0;
  const clock: { rtt: number; offset: number }[] = [];
  /** The latest round trips and when they were measured (`nowUs`), for the connection's current latency. */
  let trips: { rtt: number; at: number }[] = [];
  /** Display timestamps of the video chunks in the decoder, by sequence number, and the interval's samples. */
  const composedAt = new Map<number, number>();
  let composeToDraw: number[] = [];
  /** Video frames drawn since the last client stats. */
  let videoDrawn = 0;
  let composeToArrive: number[] = [];
  const pressed = new Set<number>();
  /** The code each physical key held here was pressed as on the remote side. */
  const sentAs = new Map<string, number>();
  /** Modifiers still held here but released on the remote side for a Mac's Option navigation (see optionAsControl):
   * pressed there again before the next physical key, so Option+Left then Option+Tab still sends Alt with Tab. */
  const lifted = new Set<number>();
  // Command travels as Control from a Mac (see remoteKey).
  const apple = isApple(navigator.platform);
  let pointer = { x: 0, y: 0 };
  /** Buttons pressed on the remote side and not released yet (an OS gesture can take a press with no `pointerup`). */
  const buttons = new Set<number>();
  const timers: ReturnType<typeof setInterval>[] = [];

  const on: Features = { ...(options.features ?? ALL_FEATURES) };
  on.recording &&= on.downloads;
  /** Whether the person steers the page: control held, and keyboard and pointer on. */
  const steers = (): boolean => on.keyboard_pointer && holdsLease();

  const renderer = createRenderer(canvas, () => fail(new Error('the WebGL context was lost'), CLOSE_CLIENT));
  renderer.onPresent = options.onPresent ?? null;
  const cursor = new Cursor(canvas, container);
  // Without the sound feature nothing plays, so no audio context is made (nor its listeners for a click to resume).
  const sound = on.sound ? new Sound((state) => options.onSound?.(state)) : null;
  if (sound === null) options.onSound?.('unsupported');
  /** Whether this connection still holds the lease of `leaseEpoch` (what a download, upload, or copy started under). */
  const holdsAt = (leaseEpoch: number): boolean => !ended && lease?.holder === LeaseHolder.You && lease.leaseEpoch === leaseEpoch;
  const downloads = new Downloads(options.session, holdsAt, (list) => options.onDownloads?.(list));
  const uploads = new Uploads(send, holdsAt, (list) => options.onUploads?.(list));
  const copies = new Copies(
    holdsAt,
    (text) => navigator.clipboard.writeText(text),
    (offer) => options.onCopied?.(offer),
  );
  const resizeObserver = new ResizeObserver(() => cursor.place());
  resizeObserver.observe(canvas);

  function send(m: ClientMessage): void {
    if (socket?.readyState !== WebSocket.OPEN) return;
    const feature = FEATURE_OF[m.type];
    if (feature !== undefined && !on[feature]) return;
    if (socket.bufferedAmount > MAX_SEND_BUFFER) {
      fail(new Error('the connection stopped draining'), CLOSE_STALLED);
      return;
    }
    socket.send(encode(m));
  }

  /** Media acknowledgements and keyframe requests: over the direct path's session while one is open. */
  function sendMedia(m: MessageOf<'media-ack'> | MessageOf<'keyframe-request'>): void {
    const session = attempt?.session;
    if (session !== undefined && session !== null) session.send(m);
    else send(m);
  }

  function requestKeyframe(epoch: number | null): void {
    if (epoch !== null) sendMedia({ type: 'keyframe-request', epoch });
  }

  /** Asks for the direct path, when this browser can open one and has none; again later unless an offer comes. */
  function askForPath(): void {
    retry = null;
    if (ended || attempt !== null || !('WebTransport' in globalThis)) return;
    send({ type: 'udp-request' });
    askLater();
  }

  function askLater(): void {
    if (!ended && retry === null) retry = setTimeout(askForPath, RETRY_MS);
  }

  function onOffer(offer: MessageOf<'udp-offer'>): void {
    if (attempt !== null) return;
    const mine: Attempt = { session: null, closed: false };
    attempt = mine;
    if (retry !== null) clearTimeout(retry);
    retry = null;
    // From now on: the server may move the media onto the session as soon as it admits it, before this side's opening
    // has finished, so what comes over it waits from the start.
    handover.opened(offer.generation);
    void openSession(offer, {
      media: (m, size) => {
        if (attempt !== mine) return;
        bytes += size;
        const now = handover.fromPath(m, size);
        // Too much waited for the WebSocket's word: the session is given up, and the server falls back.
        if (now === null) endAttempt(mine);
        else for (const message of now) take(message);
      },
      sound: (m, size) => {
        if (attempt !== mine) return;
        bytes += size;
        take(m);
      },
      closed: () => endAttempt(mine),
    }).then((opened) => {
      if (opened === null || attempt !== mine || mine.closed || ended) {
        opened?.close();
        endAttempt(mine);
        return;
      }
      mine.session = opened;
    });
  }

  /** Ends an attempt (its session closes, a fallback or this side's ending ends it), once. */
  function endAttempt(mine: Attempt): void {
    if (mine.closed) return;
    mine.closed = true;
    mine.session?.close();
    if (attempt !== mine) return;
    attempt = null;
    // Moved, the media stop until the server's fallback, which says what to take next; otherwise nothing changes.
    if (!handover.moved) handover.closed();
    askLater();
  }

  /**
   * One server message, from either path: every video, tiles and cursor message is acknowledged as it arrives,
   * whatever becomes of it (the display sends them only while what it sent and has not seen acknowledged is young and
   * small enough for this connection).
   */
  function take(m: ServerMessage): void {
    switch (m.type) {
      case 'udp-offer':
        return onOffer(m);
      case 'media-moved':
        for (const waiting of handover.movedTo(m.generation)) take(waiting);
        return;
      case 'media-fallback':
        if (attempt !== null) endAttempt(attempt);
        handover.fellBack();
        askLater();
        return;
      case 'video':
      case 'tiles':
      case 'cursor':
        sendMedia({ type: 'media-ack', revision: m.revision });
        break;
    }
    if (handover.takes(m)) handle(m);
  }

  function teardown(): void {
    ended = true;
    for (const t of timers) clearInterval(t);
    timers.length = 0;
    if (retry !== null) clearTimeout(retry);
    if (attempt !== null) endAttempt(attempt);
    video?.reset();
    video = null;
    sound?.close();
    copies.withdraw();
    downloads.stop();
    uploads.stop();
    openRequest?.close();
    lease = null;
    forgetKeys();
  }

  function fail(error: Error, code: number): void {
    if (ended) return;
    teardown();
    socket?.close(code);
    options.onError?.(error, true);
  }

  let recordings = 0;
  /** Recording needs the lease, not control: a person watching in view only records too. */
  function recordable(): boolean {
    return on.recording && lease?.holder === LeaseHolder.You && socket?.readyState === WebSocket.OPEN;
  }
  /** `ms` of `performance.now` on the display's clock, in microseconds; null before the clock is known. */
  function serverUs(ms: number): bigint | null {
    const offset = bestClock()?.offset;
    return offset === undefined ? null : BigInt(Math.max(0, Math.round(ms * 1000 + offset)));
  }

  // Input: one fence for every input message.
  function holdsLease(): boolean {
    return control && lease?.holder === LeaseHolder.You && socket?.readyState === WebSocket.OPEN;
  }

  let batches = 0;
  /** A progress listener for one new batch of uploads, which `onTransfer` hears with the batch's number and place. */
  function transferTo(at: Transfer['at']): (p: BatchProgress) => void {
    const batch = ++batches;
    return (p) => options.onTransfer?.({ ...p, batch, at });
  }

  /** Whether input started at `leaseEpoch` (a drop, a file choice) may still go on: same lease, control still on. */
  function stillHolds(leaseEpoch: number): boolean {
    return control && lease?.leaseEpoch === leaseEpoch;
  }

  function sendInput(build: (leaseEpoch: number) => ClientMessage): void {
    if (!holdsLease() || lease === null) return;
    send(build(lease.leaseEpoch));
  }

  /** Sends a press or release of the physical key `code`. */
  function sendKey(kind: typeof KeyKind.Down | typeof KeyKind.Up, code: number): void {
    sendInput((leaseEpoch) => ({ type: 'key', leaseEpoch, kind, code, text: '' }));
  }

  function sendText(text: string): void {
    for (const chunk of textChunks(text)) {
      sendInput((leaseEpoch) => ({ type: 'key', leaseEpoch, kind: KeyKind.Text, code: 0, text: chunk }));
    }
  }

  function position(e: MouseEvent): { x: number; y: number } {
    const screen = order.screen;
    if (screen === null) return pointer;
    pointer = toOutput(e.clientX, e.clientY, canvas.getBoundingClientRect(), screen);
    return pointer;
  }

  function onPointer(event: PointerKind) {
    return (e: PointerEvent): void => {
      if (!steers() || e.button > 2) return;
      const { x, y } = position(e);
      // A button pressed or released while another is held comes as a move (a chord): it travels as its press or release.
      const chord = event === PointerKind.Move && e.button >= 0 ? (e.buttons & BUTTON_BITS[e.button]!) !== 0 : null;
      const kind = chord === null || chord === buttons.has(e.button) ? event : chord ? PointerKind.Down : PointerKind.Up;
      if (kind === PointerKind.Down) {
        // Option lifted for a Mac's word navigation, and still held, goes with the click (Option-click).
        restoreLifted(e);
        syncModifiers(e);
      }
      if (kind === PointerKind.Move) {
        if ((socket?.bufferedAmount ?? 0) > MOVE_SEND_BUDGET) return;
      } else {
        e.preventDefault();
        if (kind === PointerKind.Down) {
          // Capture keeps the button's release on the screen; a pointer the browser no longer tracks cannot be captured.
          try {
            canvas.setPointerCapture(e.pointerId);
          } catch {
            // The press still counts without capture.
          }
          keyboard.focus({ preventScroll: true });
        }
      }
      const button = kind === PointerKind.Move ? 0 : e.button;
      if (kind === PointerKind.Down) buttons.add(button);
      if (kind === PointerKind.Up) buttons.delete(button);
      sendInput((leaseEpoch) => ({ type: 'pointer', leaseEpoch, kind, button, x, y }));
    };
  }

  /** Releases, where the pointer last was, the buttons a cancelled press or a lost capture left held. */
  function releaseButtons(): void {
    const { x, y } = pointer;
    for (const button of buttons) sendInput((leaseEpoch) => ({ type: 'pointer', leaseEpoch, kind: PointerKind.Up, button, x, y }));
    buttons.clear();
  }

  canvas.addEventListener('pointermove', onPointer(PointerKind.Move));
  canvas.addEventListener('pointerdown', onPointer(PointerKind.Down));
  canvas.addEventListener('pointerup', onPointer(PointerKind.Up));
  // A cancelled press has no `pointerup` (and its `button` is -1); a capture lost after a release finds nothing held.
  canvas.addEventListener('pointercancel', releaseButtons);
  canvas.addEventListener('lostpointercapture', releaseButtons);
  // Files dragged over the screen are dragged over the page too, so it shows its drag-over state; they drop where
  // the person let go once uploaded, the page's drag held there meanwhile. Each drag over the screen is a new
  // generation: a drop still uploading never ends a newer drag's hold, though it still drops when its files arrive,
  // and then the newest drop still uploading is held again.
  let dragAt: { x: number; y: number; at: number } | null = null;
  let dragGeneration = 0;
  let held: { generation: number; x: number; y: number; types: string[] } | null = null;
  const dragTypes = (data: DataTransfer): string[] => {
    const types = [...data.items]
      .filter((item) => item.kind === 'file')
      .slice(0, MAX_CHOSEN)
      .map((item) => (isMediaType(item.type.toLowerCase()) ? item.type.toLowerCase() : ''));
    return types.length > 0 ? types : [''];
  };
  const endDrag = (leaseEpoch: number, generation: number): void => {
    if (generation === dragGeneration && lease?.leaseEpoch === leaseEpoch) send({ type: 'file-drag-end', leaseEpoch });
  };
  // Every drag over the screen is cancelled, and one it does not take (view only, no uploads, a link) is refused:
  // otherwise the browser would open the dropped file or link in place of this page.
  canvas.addEventListener('dragover', (e) => {
    e.preventDefault();
    if (!on.uploads || !holdsLease() || lease === null || !e.dataTransfer?.types.includes('Files')) {
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'none';
      return;
    }
    e.dataTransfer.dropEffect = 'copy';
    const { x, y } = position(e);
    const at = performance.now();
    if (dragAt === null) dragGeneration += 1;
    // Unmoved, the drag is repeated now and then, so a hold a finishing drop interrupted comes back.
    else if (dragAt.x === x && dragAt.y === y && at - dragAt.at < 250) return;
    dragAt = { x, y, at };
    send({ type: 'file-drag', leaseEpoch: lease.leaseEpoch, x, y, types: dragTypes(e.dataTransfer) });
  });
  canvas.addEventListener('dragleave', () => {
    if (dragAt === null || lease === null) return;
    dragAt = null;
    endDrag(lease.leaseEpoch, dragGeneration);
  });
  canvas.addEventListener('drop', (e) => {
    e.preventDefault();
    if (!on.uploads || !holdsLease() || lease === null || e.dataTransfer === null) return;
    dragAt = null;
    const files = [...e.dataTransfer.files];
    const { x, y } = position(e);
    const leaseEpoch = lease.leaseEpoch;
    const generation = dragGeneration;
    held = { generation, x, y, types: dragTypes(e.dataTransfer) };
    const settled = (): void => {
      if (held?.generation === generation) held = null;
    };
    const box = container.getBoundingClientRect();
    uploads.upload(files, leaseEpoch, transferTo({ x: e.clientX - box.left, y: e.clientY - box.top })).then(
      (chosen) => {
        settled();
        if (chosen.length === 0) return endDrag(leaseEpoch, generation);
        if (!stillHolds(leaseEpoch)) return endDrag(leaseEpoch, generation);
        send({ type: 'file-drop', leaseEpoch, x, y, uploads: chosen });
        // The display holds this drag once the drop is done.
        if (held !== null) send({ type: 'file-drag', leaseEpoch, x: held.x, y: held.y, types: held.types });
      },
      () => {
        settled();
        endDrag(leaseEpoch, generation);
      },
    );
  });
  canvas.addEventListener('contextmenu', (e) => {
    if (steers()) e.preventDefault();
  });
  canvas.addEventListener(
    'wheel',
    (e) => {
      const screen = order.screen;
      if (!steers() || screen === null) return;
      e.preventDefault();
      // As with a press: Control held (Control-wheel zooms) goes with the wheel, and the keys' release reaches this page.
      // A modifier only the wheel says is held goes with it alone: a trackpad's pinch comes as a Control-wheel with no
      // Control key to release.
      restoreLifted(e);
      const held = new Set(pressed);
      syncModifiers(e);
      keyboard.focus({ preventScroll: true });
      const { x, y } = position(e);
      const dx = wheelPixels(e.deltaX, e.deltaMode, screen.width);
      const dy = wheelPixels(e.deltaY, e.deltaMode, screen.height);
      if (dx !== 0 || dy !== 0) sendInput((leaseEpoch) => ({ type: 'wheel', leaseEpoch, x, y, dx, dy }));
      for (const code of pressed) {
        if (held.has(code)) continue;
        pressed.delete(code);
        sendKey(KeyKind.Up, code);
      }
    },
    { passive: false },
  );

  keyboard.addEventListener('keydown', (e) => {
    // With the clipboard on, the paste shortcut is left to the browser, whose paste event sends the person's clipboard;
    // with it off, the shortcut travels as keys, so the remote page pastes its own clipboard.
    if (!steers() || feedsComposition(e) || (on.clipboard && isPasteShortcut(e, apple))) return;
    // A modifier held before this page had the focus (Control held while pressing Take control) goes with the key.
    if (!isModifier(e.code)) syncModifiers(e);
    // A printable character travels as text, so what appears does not depend on the remote keyboard layout.
    // Shortcuts (see isShortcut) and keys without a character travel as key codes (see travelsAs).
    const character = keyText(e.key);
    // Shift+Space travels as keys: as text, its Shift would be dropped (a page scrolls back up with it).
    if (character !== null && [...character].length === 1 && !isShortcut(e, apple) && !(e.shiftKey && e.code === 'Space')) {
      e.preventDefault();
      // AltGr (on Windows also a Control) or a Mac's Option stays held on the remote side: the display types text with
      // no modifier on it, whatever is held (display/src/input.rs, `tap_alone`).
      sendText(character);
      return;
    }
    const code = evdevCode(remoteKey(travelsAs(e), apple));
    if (code === undefined) {
      if (character === null) return;
      e.preventDefault();
      sendText(character);
      return;
    }
    e.preventDefault();
    // Option's Alt is released on the remote side and Control goes with the key, pressed and released with it: the
    // remote side repeats no key, so each of this browser's repeats moves or deletes one more word.
    if (optionAsControl(e, apple)) {
      liftKeys(OPTION_KEYS);
      if (pressed.delete(code)) sendKey(KeyKind.Up, code);
      sendKey(KeyKind.Down, OPTION_CONTROL);
      sendKey(KeyKind.Down, code);
      sendKey(KeyKind.Up, code);
      sendKey(KeyKind.Up, OPTION_CONTROL);
      return;
    }
    restoreLifted(e);
    // On a Mac, a key pressed while Command is held never reports its release: it is pressed and released at once.
    if (apple && e.metaKey && !isModifier(e.code)) {
      sendKey(KeyKind.Down, code);
      sendKey(KeyKind.Up, code);
      return;
    }
    // The remote side repeats no key: this browser's repeats travel (see keyDown).
    const action = keyDown(e.code, e.repeat, pressed.has(code));
    if (action === 'none') return;
    if (action === 'repeat') sendKey(KeyKind.Up, code);
    // The same physical key held as another code (Num Lock changed during a repeat): that one is released first.
    const before = sentAs.get(e.code);
    if (before !== undefined && before !== code && pressed.delete(before)) sendKey(KeyKind.Up, before);
    pressed.add(code);
    sentAs.set(e.code, code);
    sendKey(KeyKind.Down, code);
  });
  keyboard.addEventListener('keyup', (e) => {
    // Released as it was pressed: a keypad key's Num Lock may have changed meanwhile.
    const code = sentAs.get(e.code) ?? evdevCode(remoteKey(travelsAs(e), apple));
    sentAs.delete(e.code);
    if (code !== undefined) lifted.delete(code);
    if (code !== undefined && pressed.delete(code)) {
      e.preventDefault();
      sendKey(KeyKind.Up, code);
    }
    // A modifier pressed for a click (see syncModifiers) goes with the release of either side.
    if (steers()) syncModifiers(e);
  });
  keyboard.addEventListener('blur', () => {
    releaseKeys();
    lifted.clear();
  });
  keyboard.addEventListener('compositionend', (e) => {
    if (steers() && e.data) sendText(e.data);
    keyboard.value = '';
  });
  // A virtual keyboard's Backspace or Return may come only as an editing intent (its key events say keyCode 229): it
  // travels as that key, pressed and released. A key handled above was prevented, so no intent follows it.
  keyboard.addEventListener('beforeinput', (e) => {
    const code = EDIT_KEYS[e.inputType];
    if (code === undefined || e.isComposing || !steers()) return;
    e.preventDefault();
    sendKey(KeyKind.Down, code);
    sendKey(KeyKind.Up, code);
  });
  keyboard.addEventListener('input', (e) => {
    // Composition results arrive with compositionend; plain insertions come from virtual keyboards.
    if (e instanceof InputEvent && e.inputType === 'insertText' && !e.isComposing && e.data && steers()) {
      sendText(e.data);
    }
    if (!(e instanceof InputEvent && e.isComposing)) keyboard.value = '';
  });
  keyboard.addEventListener('paste', (e) => {
    if (!on.clipboard || !holdsLease()) return;
    e.preventDefault();
    const text = e.clipboardData?.getData('text/plain') ?? '';
    if (text === '') return;
    if (utf8Length(text) > MAX_CLIPBOARD) {
      options.onError?.(new Error(`pasted text is larger than ${MAX_CLIPBOARD} bytes`), false);
      return;
    }
    sendInput((leaseEpoch) => ({ type: 'clipboard', leaseEpoch, text }));
  });

  /** Makes the remote side's modifiers what an event reports held: one held but never pressed here (held while another
   * control had the focus) is pressed, so Control-click is one, unless lifted for a Mac's Option navigation; one released,
   * either side, is released, whichever side was pressed for it. */
  function syncModifiers(e: MouseEvent | KeyboardEvent): void {
    const held: [boolean, ReadonlySet<number>, string][] = [
      [e.ctrlKey || (apple && e.metaKey), CONTROL_KEYS, 'ControlLeft'],
      [e.shiftKey, SHIFT_KEYS, 'ShiftLeft'],
      [e.altKey, OPTION_KEYS, 'AltLeft'],
    ];
    for (const [on, codes, key] of held) {
      const down = [...codes].filter((code) => pressed.has(code));
      if (!on) {
        for (const code of down) {
          pressed.delete(code);
          sendKey(KeyKind.Up, code);
        }
      } else if (down.length === 0 && ![...codes].some((code) => lifted.has(code))) {
        const code = evdevCode(key)!;
        pressed.add(code);
        sendKey(KeyKind.Down, code);
      }
    }
  }

  /** Releases the held modifiers in `only` on the remote side, remembering them as still held here. */
  function liftKeys(only: ReadonlySet<number>): void {
    for (const code of pressed) if (only.has(code)) lifted.add(code);
    releaseKeys(only);
  }

  /** Presses again on the remote side the lifted modifiers this key or pointer event still reports held. */
  function restoreLifted(e: MouseEvent | KeyboardEvent): void {
    for (const code of lifted) {
      lifted.delete(code);
      if (pressed.has(code) || !e.altKey) continue;
      pressed.add(code);
      sendKey(KeyKind.Down, code);
    }
  }

  /** Forgets what is held on the remote side, as the display does at each new lease (or as the page ends). */
  function forgetKeys(): void {
    pressed.clear();
    lifted.clear();
    sentAs.clear();
    buttons.clear();
  }

  /** Releases the held keys (only those in `only`, when given) on the remote side. */
  function releaseKeys(only?: ReadonlySet<number>): void {
    for (const code of pressed) {
      if (only && !only.has(code)) continue;
      sendKey(KeyKind.Up, code);
      pressed.delete(code);
    }
  }

  // Server messages.
  function onConfig(m: MessageOf<'config'>): void {
    const result = order.config(m, performance.now());
    if (result.verdict !== 'reset') return;
    renderer.resize(m.width, m.height);
    pictured = false;
    // For a page that sizes the screen by its shape (`--screen-ratio`), larger than its pixels included.
    canvas.style.setProperty('--screen-ratio', String(m.width / m.height));
    cursor.reset();
    video?.reset();
    video = m.codec === Codec.None ? null : new VideoStream(m.codec, { output: onFrame, error: onDecodeError });
  }

  function onFrame(frame: VideoFrame, seq: number): void {
    renderer.drawVideo(
      frame,
      () => order.videoOutput(seq),
      (placed) => {
        const at = composedAt.get(seq);
        composedAt.delete(seq);
        const offset = bestClock()?.offset;
        if (placed) {
          videoDrawn++;
          picture();
        }
        if (placed && at !== undefined && offset !== undefined && composeToDraw.length < 1024) {
          composeToDraw.push(nowUs() + offset - at);
        }
      },
    );
  }

  function onDecodeError(error: Error): void {
    options.onError?.(error, false);
    requestKeyframe(order.decoderLost(performance.now()));
  }

  function onVideo(m: MessageOf<'video'>): void {
    mediaBytes += m.payload.length;
    const verdict = order.video(m, performance.now());
    if (verdict.kind === 'drop') {
      requestKeyframe(verdict.keyframeRequest);
      return;
    }
    if (video === null) return;
    if (composedAt.size >= 256) composedAt.clear();
    composedAt.set(verdict.seq, Number(m.timestampUs));
    const offset = bestClock()?.offset;
    if (offset !== undefined && composeToArrive.length < 1024) composeToArrive.push(nowUs() + offset - Number(m.timestampUs));
    video.decode(m.payload, verdict.keyframe, verdict.seq);
    unstick();
  }

  /** A decoder stuck behind its input is dropped, and the next keyframe asked for. */
  function unstick(): void {
    const now = performance.now();
    if (video === null || !video.stuck(now)) return;
    video.reset();
    requestKeyframe(order.decoderLost(now));
  }

  function onTiles(m: MessageOf<'tiles'>): void {
    mediaBytes += m.data.length;
    if (order.tiles(m) !== 'apply') return;
    const images = decodeTiles(m.rects, m.rawLen, m.data);
    m.rects.forEach((rect, i) => renderer.drawTile(rect, images[i]!));
    if (m.rects.length > 0) picture();
  }

  /** Whether the canvas has drawn anything since its configuration (see `onPicture`). */
  let pictured = false;
  function picture(): void {
    if (pictured) return;
    pictured = true;
    options.onPicture?.();
  }

  function onPong(m: MessageOf<'time-pong'>): void {
    const now = nowUs();
    const sent = Number(m.clientTimeUs);
    const rtt = now - sent;
    if (!(rtt >= 0 && rtt <= PING_MS * 1000 * CLOCK_SAMPLES)) return;
    clock.push({ rtt, offset: Number(m.serverTimeUs) - (sent + rtt / 2) });
    if (clock.length > CLOCK_SAMPLES) clock.shift();
    trips = [...trips, { rtt, at: now }].slice(-RECENT_TRIPS);
  }

  /** The median recent round trip: samples older than a few pings no longer describe the connection. */
  function roundTrip(): number | null {
    const fresh = trips.filter((t) => nowUs() - t.at <= RECENT_TRIPS * PING_MS * 1000).map((t) => t.rtt);
    return p50(fresh);
  }

  function onCopied(m: MessageOf<'copied'>): void {
    // Automatic writes are allowed while this page has focus (and, in Firefox and Safari, right after the person's own
    // key press or click).
    copies.arrived(m.text, m.leaseEpoch, autoCopy);
  }

  /** The page opened a file chooser: the person's own file picker opens, and what they choose goes to the page. */
  function onFileRequest(m: MessageOf<'file-request'>): void {
    const leaseEpoch = m.leaseEpoch;
    if (!on.uploads || lease?.holder !== LeaseHolder.You || lease.leaseEpoch !== leaseEpoch) return;
    // View only: the page's chooser is cancelled at once rather than left waiting.
    if (!control) return send({ type: 'file-choose', leaseEpoch, request: m.request, uploads: [] });
    // A newer request replaces an older one. Each stays open until answered, or until the page closes its chooser (then
    // nothing chosen is sent).
    openRequest?.close();
    let open = true;
    openRequest = {
      request: m.request,
      close: () => {
        open = false;
        openRequest = null;
        options.onFileRequest?.(null);
      },
      cancel: () => cancel(),
    };
    // Files chosen settle the request: their upload's end sends the answer, so nothing may choose or cancel again.
    let answered = false;
    const answer = (chosen: number[]): void => {
      if (openRequest?.request === m.request) openRequest.close();
      // Control turned off meanwhile (view only): the chooser is cancelled rather than left waiting.
      if (lease?.leaseEpoch === leaseEpoch) send({ type: 'file-choose', leaseEpoch, request: m.request, uploads: control ? chosen : [] });
    };
    const cancel = (): void => {
      if (open && !answered) answer([]);
    };
    const picker = document.createElement('input');
    picker.type = 'file';
    picker.multiple = m.multiple;
    picker.addEventListener('change', () => {
      if (!open || answered) return;
      answered = true;
      options.onFileRequest?.(null);
      const files = [...(picker.files ?? [])];
      void uploads.upload(files, leaseEpoch, transferTo(null)).then(answer, () => answer([]));
    });
    picker.addEventListener('cancel', cancel);
    // The person's click on the page opened the chooser a moment ago; that click lets this page open its picker too.
    if (navigator.userActivation?.isActive) {
      picker.click();
    } else {
      options.onFileRequest?.({
        choose: () => {
          if (!answered) picker.click();
        },
        cancel,
      });
    }
  }

  function onLease(m: MessageOf<'lease'>): void {
    if (lease?.holder === LeaseHolder.You && m.holder !== LeaseHolder.You) forgetKeys();
    if (lease?.leaseEpoch !== m.leaseEpoch) {
      // A new lease, even this viewer's again after a takeover: the display released every key and button.
      forgetKeys();
      downloads.stop(m.holder === LeaseHolder.You ? m.leaseEpoch : undefined);
      copies.withdraw();
      uploads.stop();
      openRequest?.close();
      dragAt = null;
      held = null;
    }
    lease = { leaseEpoch: m.leaseEpoch, holder: m.holder };
    if (m.holder === LeaseHolder.You) downloads.leaseHeld(m.leaseEpoch);
    options.onLease?.(lease);
  }

  function handle(m: ServerMessage): void {
    switch (m.type) {
      case 'config':
        return onConfig(m);
      case 'video':
        return onVideo(m);
      case 'tiles':
        return onTiles(m);
      case 'copied':
        if (on.clipboard) onCopied(m);
        return;
      case 'file-offer':
        if (on.downloads) downloads.offer(m);
        return;
      case 'file-link':
        if (on.downloads) downloads.link(m);
        return;
      case 'file-end':
        if (on.downloads) downloads.end(m);
        return;
      case 'file-gone':
        if (on.downloads) downloads.gone(m);
        return;
      case 'upload-ack':
        return uploads.ack(m);
      case 'upload-done':
        return uploads.done(m);
      case 'file-request':
        return onFileRequest(m);
      case 'file-request-end':
        if (openRequest?.request === m.request) openRequest.close();
        return;
      case 'audio':
        mediaBytes += m.payload.length;
        return sound?.push(m, nowUs(), bestClock()?.offset);
      case 'cursor':
        if (order.cursor(m) === 'apply') cursor.update(m.x, m.y, m.shape, m.hotX, m.hotY, m.image);
        return;
      case 'server-stats':
        lastServer = m;
        return;
      case 'time-pong':
        return onPong(m);
      case 'lease':
        return onLease(m);
      case 'record-state':
        if (!on.recording) return;
        return options.onRecording?.({ recording: m.recording, phase: m.phase, file: m.file, seconds: m.seconds, bytes: m.bytes });
    }
  }

  /** The clock sample with the shortest round trip, the tightest offset bound. */
  function bestClock(): { rtt: number; offset: number } | null {
    return clock.reduce<{ rtt: number; offset: number } | null>((a, s) => (a === null || s.rtt < a.rtt ? s : a), null);
  }

  function stats(): void {
    const best = bestClock();
    const drawn = p50(composeToDraw);
    composeToDraw = [];
    const arrived = p50(composeToArrive);
    composeToArrive = [];
    const codec = order.screen?.codec ?? null;
    const hardware = codec === null ? null : (probe.hardware[codec] ?? null);
    const presented = renderer.takePresented();
    const decodeUsP50 = video?.takeDecodeP50() ?? 0;
    // The server compares video frames drawn with video frames sent: tiles drawn between them say nothing of decoding.
    send({ type: 'client-stats', presented: videoDrawn, decodeUsP50, hardware: hardware === true });
    videoDrawn = 0;
    options.onStats?.({
      presented,
      decodeUsP50,
      codec,
      hardware,
      renderer: renderer.kind,
      composeToDrawUsP50: drawn,
      composeToArriveUsP50: arrived,
      mediaBytes,
      bytes,
      clockOffsetUs: best === null ? null : Math.round(best.offset),
      clockErrorUs: best === null ? null : Math.round(best.rtt / 2),
      roundTripUs: roundTrip(),
      server: lastServer,
      transport: attempt?.session != null && handover.moved ? 'udp' : 'websocket',
    });
    bytes = 0;
    mediaBytes = 0;
  }

  function connect(): void {
    const ws = new WebSocket(options.url);
    socket = ws;
    ws.binaryType = 'arraybuffer';
    ws.onopen = () => {
      if (ended) return;
      send({ type: 'hello', version: VERSION, codecs: probe.codecs });
      const ping = (): void => send({ type: 'time-ping', clientTimeUs: BigInt(Math.round(nowUs())) });
      ping();
      timers.push(setInterval(ping, PING_MS), setInterval(stats, STATS_MS), setInterval(unstick, WATCHDOG_MS));
      askForPath();
    };
    ws.onmessage = (event: MessageEvent<unknown>) => {
      if (ended) return;
      try {
        if (!(event.data instanceof ArrayBuffer)) throw new Error('a text message on a binary stream');
        bytes += event.data.byteLength;
        take(decode(new Uint8Array(event.data)));
      } catch (e) {
        fail(e instanceof Error ? e : new Error(String(e)), CLOSE_PROTOCOL);
      }
    };
    ws.onerror = () => fail(new Error('the connection failed'), CLOSE_NORMAL);
    ws.onclose = (event) =>
      fail(
        Object.assign(new Error(`the connection closed (${event.code}${event.reason ? `: ${event.reason}` : ''})`), { closeCode: event.code }),
        CLOSE_NORMAL,
      );
  }

  probeCodecs()
    .then((result) => {
      probe = result;
      if (!ended) connect();
    })
    .catch((e: unknown) => fail(e instanceof Error ? e : new Error(String(e)), CLOSE_CLIENT));

  return {
    writeCopied(): Promise<void> {
      return copies.writeOffered();
    },
    setAutoCopy(enabled: boolean): void {
      autoCopy = enabled;
    },
    setMuted(muted: boolean): void {
      sound?.setMuted(muted);
    },
    setVolume(volume: number): void {
      sound?.setVolume(volume);
    },
    startRecording(request: RecordingRequest): number | null {
      if (!recordable() || lease === null) return null;
      const recording = ++recordings;
      // A browser that cannot send the microphone (Firefox, Safari) never tells the display to wait for one.
      const microphone = request.microphone && microphoneSupported();
      // The page's sound only where the session has it (the admin records none otherwise).
      send({ type: 'record-start', leaseEpoch: lease.leaseEpoch, recording, ...request, microphone, pageSound: request.pageSound && on.sound });
      // Asking can end the connection (it stopped draining): then nothing started.
      return recordable() ? recording : null;
    },
    recordCamera(id: number, jpeg: Uint8Array, takenMs: number): boolean {
      const at = serverUs(takenMs);
      if (!recordable() || lease === null || at === null || (socket?.bufferedAmount ?? 0) > RECORD_BUFFER) return false;
      send({ type: 'record-camera', leaseEpoch: lease.leaseEpoch, recording: id, timestampUs: at, jpeg });
      return true;
    },
    recordMicrophone(id: number, packet: Uint8Array, startedMs: number): void {
      const at = serverUs(startedMs);
      if (!recordable() || lease === null || at === null) return;
      send({ type: 'record-microphone', leaseEpoch: lease.leaseEpoch, recording: id, timestampUs: at, payload: packet });
    },
    recordLayout(id: number, layout: CameraLayout): void {
      if (recordable() && lease !== null) send({ type: 'record-layout', leaseEpoch: lease.leaseEpoch, recording: id, layout });
    },
    stopRecording(id: number): void {
      if (recordable() && lease !== null) send({ type: 'record-stop', leaseEpoch: lease.leaseEpoch, recording: id });
    },
    setControl(enabled: boolean): void {
      // View only: what was held is released, and a chooser the page has open is answered with nothing rather than
      // left waiting behind hidden buttons.
      if (!enabled) {
        releaseKeys();
        releaseButtons();
        openRequest?.cancel();
      }
      // Focus only when control turns on: the page re-announces the lease to every viewer whenever any connects, and
      // focusing then would pull the person's typing (or an open menu) into the remote page. And only while this page
      // has focus: a takeover in a frame the person never touched must not pull the framing page's keys into it (a
      // press on the screen still focuses it).
      const turnedOn = enabled && !control;
      control = enabled;
      if (turnedOn && document.hasFocus()) keyboard.focus({ preventScroll: true });
    },
    close(): void {
      if (unmounted) return;
      unmounted = true;
      if (!ended) {
        teardown();
        socket?.close(1000);
      }
      resizeObserver.disconnect();
      renderer.destroy();
      cursor.destroy();
      canvas.remove();
      keyboard.remove();
    },
  };
}
