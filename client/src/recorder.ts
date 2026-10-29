// Recording, the person's side: the display records the screen itself, at its own quality (see display/src/recording.rs);
// this side picks the devices, sends the camera as JPEG pictures and the microphone as Opus packets while it records,
// and shows where the camera will be. The recording reaches the person as an ordinary download that grows as it is
// made.

import {
  AUDIO_SAMPLE_RATE,
  CAMERA_RADIUS_MAX,
  CAMERA_SIDE_MAX,
  CAMERA_SIDE_MIN,
  type CameraLayout as WireLayout,
  LAYOUT_UNITS,
  MAX_RECORD_PICTURE,
  type Rect,
} from './protocol.ts';
import type { Viewer } from './viewer.ts';

/**
 * Where the camera is, and how it looks. It is a square: `x` and `y` place its top-left corner, as shares of the
 * screen's width and height; `side` is its side, as a share of the screen's width (from `SIDE_MIN` to `SIDE_MAX`);
 * `radius` rounds its corners, in percent of half its side (0 square, 100 a circle).
 */
export interface CameraLayout {
  x: number;
  y: number;
  side: number;
  radius: number;
}

/** The camera's smallest and largest side, as shares of the screen's width. */
export const SIDE_MIN = CAMERA_SIDE_MIN / LAYOUT_UNITS;
export const SIDE_MAX = CAMERA_SIDE_MAX / LAYOUT_UNITS;

/** Bottom right, a little in from the edges, on a 16:10 screen. */
export const DEFAULT_LAYOUT: CameraLayout = { x: 0.74, y: 0.58, side: 0.24, radius: 24 };

/** Camera pictures a second, and the largest side of the centred square sent (the display draws it covering the camera). */
const CAMERA_FPS = 15;
const CAMERA_MAX_SIDE = 480;
/** JPEG qualities tried, best first, until a picture fits a message. */
const QUALITIES = [0.75, 0.6, 0.45];
/** The microphone's Opus rate. */
const MICROPHONE_BITS = 96_000;

/** Where the camera goes on a screen `width` by `height`, in the same units: a square no taller than the screen, placed as asked, kept inside. */
export function cameraRect(width: number, height: number, layout: CameraLayout): Rect {
  const side = Math.min(Math.round(sideOnWire(layout.side) * width / LAYOUT_UNITS), height);
  const x = Math.min(Math.max(Math.round(onWire(layout.x) * width / LAYOUT_UNITS), 0), Math.max(width - side, 0));
  const y = Math.min(Math.max(Math.round(onWire(layout.y) * height / LAYOUT_UNITS), 0), Math.max(height - side, 0));
  return { x, y, w: side, h: side };
}

/** `layout` moved by `dx` and `dy` (shares of the screen), its position kept where the camera is still inside. */
export function moveCamera(width: number, height: number, layout: CameraLayout, dx: number, dy: number): CameraLayout {
  if (width === 0 || height === 0) return layout;
  const r = cameraRect(width, height, layout);
  const x = Math.min(Math.max(r.x / width + dx, 0), Math.max(1 - r.w / width, 0));
  const y = Math.min(Math.max(r.y / height + dy, 0), Math.max(1 - r.h / height, 0));
  return { ...layout, x, y };
}

/** The camera's corner that stays put while it is resized: the one toward the nearest screen edges (`right`, `bottom`). */
export interface KeptCorner {
  right: boolean;
  bottom: boolean;
}

/** The corner a resize keeps: the camera grows toward the middle of the screen. */
export function keptCorner(width: number, height: number, layout: CameraLayout): KeptCorner {
  const r = cameraRect(width, height, layout);
  return { right: r.x + r.w / 2 > width / 2, bottom: r.y + r.h / 2 > height / 2 };
}

/**
 * `layout` resized to `side` (a share of the screen's width) with its `kept` corner where it is: the side stays within
 * `SIDE_MIN` and `SIDE_MAX`, no taller than the screen, and inside the screen from that corner.
 */
export function resizeCamera(width: number, height: number, layout: CameraLayout, side: number, kept: KeptCorner): CameraLayout {
  if (width === 0 || height === 0) return layout;
  const r = cameraRect(width, height, layout);
  const cornerX = kept.right ? r.x + r.w : r.x;
  const cornerY = kept.bottom ? r.y + r.h : r.y;
  const room = Math.min(kept.right ? cornerX : width - cornerX, kept.bottom ? cornerY : height - cornerY, height);
  const s = Math.max(Math.min(side, SIDE_MAX, room / width), SIDE_MIN);
  const x = kept.right ? (cornerX - s * width) / width : cornerX / width;
  const y = kept.bottom ? (cornerY - s * width) / height : cornerY / height;
  return { ...layout, side: s, x: Math.min(Math.max(x, 0), 1), y: Math.min(Math.max(y, 0), 1) };
}

/** A layout read back from `text` (as `JSON.stringify` wrote it), or null when it is not one. */
export function parseLayout(text: string | null): CameraLayout | null {
  try {
    const v = JSON.parse(text ?? 'null') as Partial<CameraLayout> | null;
    const share = (n: unknown) => typeof n === 'number' && n >= 0 && n <= 1;
    if (
      v === null ||
      !share(v.x) ||
      !share(v.y) ||
      !(typeof v.side === 'number' && v.side >= SIDE_MIN && v.side <= SIDE_MAX) ||
      !(Number.isInteger(v.radius) && v.radius! >= 0 && v.radius! <= CAMERA_RADIUS_MAX)
    ) {
      return null;
    }
    return { x: v.x!, y: v.y!, side: v.side, radius: v.radius! };
  } catch {
    return null;
  }
}

/** A share of the screen in the wire's steps. */
function onWire(share: number): number {
  return Math.min(Math.max(Math.round(share * LAYOUT_UNITS), 0), LAYOUT_UNITS);
}

/** A side in the wire's steps, within the protocol's bounds. */
function sideOnWire(share: number): number {
  return Math.min(Math.max(Math.round(share * LAYOUT_UNITS), CAMERA_SIDE_MIN), CAMERA_SIDE_MAX);
}

/** The wire's form of a layout. */
export function wire(layout: CameraLayout): WireLayout {
  const radius = Math.min(Math.max(Math.round(layout.radius), 0), CAMERA_RADIUS_MAX);
  return { x: onWire(layout.x), y: onWire(layout.y), side: sideOnWire(layout.side), radius };
}

export interface Devices {
  /** Each by its browser's name, or by its place (`number`) when the browser gives none. */
  microphones: { id: string; label: string | null; number: number }[];
  cameras: { id: string; label: string | null; number: number }[];
}

/** The microphones and cameras; their names show only once the person allowed one of each kind. */
export async function listDevices(): Promise<Devices> {
  const all = await navigator.mediaDevices.enumerateDevices();
  const of = (kind: MediaDeviceKind) =>
    all.filter((d) => d.kind === kind && d.deviceId !== '').map((d, i) => ({ id: d.deviceId, label: d.label || null, number: i + 1 }));
  return { microphones: of('audioinput'), cameras: of('videoinput') };
}

/** Opens one device (`default` for the browser's choice); the caller stops its tracks. */
export function openDevice(kind: 'microphone' | 'camera', id: string): Promise<MediaStream> {
  const constraint = id === 'default' ? true : { deviceId: { exact: id } };
  return navigator.mediaDevices.getUserMedia(kind === 'camera' ? { video: constraint } : { audio: constraint });
}

/** Whether this browser can send a microphone to a recording: it needs Chrome's track processor (not in Firefox or Safari). */
export function microphoneSupported(): boolean {
  return typeof MediaStreamTrackProcessor !== 'undefined' && typeof AudioEncoder !== 'undefined';
}

/**
 * Sends recording `id`'s camera and microphone to the display until `stop`. The devices' streams stay the caller's;
 * everything this opened (a picture loop, an audio context, an encoder) ends with `stop`.
 */
export class Capture {
  private readonly video = document.createElement('video');
  private readonly canvas = new OffscreenCanvas(1, 1);
  private timer: ReturnType<typeof setInterval> | undefined;
  private picturing = false;
  private audio: AudioContext | null = null;
  private encoder: AudioEncoder | null = null;
  private reader: ReadableStreamDefaultReader<AudioData> | null = null;
  private stopped = false;

  constructor(
    private readonly viewer: Viewer,
    private readonly id: number,
    camera: MediaStream | null,
    microphone: MediaStream | null,
  ) {
    if (camera) {
      this.video.muted = true;
      this.video.playsInline = true;
      this.video.srcObject = camera;
      void this.video.play().catch(() => undefined);
      this.timer = setInterval(() => void this.picture(), 1000 / CAMERA_FPS);
    }
    if (microphone) void this.microphone(microphone);
  }

  stop(): void {
    this.stopped = true;
    clearInterval(this.timer);
    this.video.srcObject = null;
    void this.reader?.cancel().catch(() => undefined);
    if (this.encoder && this.encoder.state !== 'closed') this.encoder.close();
    void this.audio?.close();
  }

  /** One camera picture: its centred square (the camera's shape), as a JPEG that fits. */
  private async picture(): Promise<void> {
    const v = this.video;
    if (this.stopped || this.picturing || v.readyState < 2 || v.videoWidth === 0) return;
    this.picturing = true;
    const taken = performance.now();
    try {
      const g = this.canvas.getContext('2d');
      if (!g) return;
      const crop = Math.min(v.videoWidth, v.videoHeight);
      const side = Math.max(Math.min(crop, CAMERA_MAX_SIDE), 1);
      if (this.canvas.width !== side || this.canvas.height !== side) {
        this.canvas.width = side;
        this.canvas.height = side;
      }
      g.drawImage(v, (v.videoWidth - crop) / 2, (v.videoHeight - crop) / 2, crop, crop, 0, 0, side, side);
      for (const quality of QUALITIES) {
        const blob = await this.canvas.convertToBlob({ type: 'image/jpeg', quality });
        if (blob.size <= MAX_RECORD_PICTURE) {
          if (!this.stopped) this.viewer.recordCamera(this.id, new Uint8Array(await blob.arrayBuffer()), taken);
          return;
        }
      }
    } finally {
      this.picturing = false;
    }
  }

  /** The microphone at 48 kHz (an audio context resamples it), encoded as 20 ms Opus packets. */
  private async microphone(stream: MediaStream): Promise<void> {
    if (!microphoneSupported()) return;
    const audio = new AudioContext({ sampleRate: AUDIO_SAMPLE_RATE });
    this.audio = audio;
    const out = audio.createMediaStreamDestination();
    audio.createMediaStreamSource(stream).connect(out);
    const track = out.stream.getAudioTracks()[0];
    if (!track) return;
    // Packets are stamped from the first sample's arrival on, by the encoder's own timeline.
    let first: { data: number; perf: number } | null = null;
    const encoder = new AudioEncoder({
      output: (chunk) => {
        if (this.stopped || first === null) return;
        const packet = new Uint8Array(chunk.byteLength);
        chunk.copyTo(packet);
        this.viewer.recordMicrophone(this.id, packet, first.perf + (chunk.timestamp - first.data) / 1000);
      },
      error: () => undefined,
    });
    this.encoder = encoder;
    const reader = new MediaStreamTrackProcessor({ track }).readable.getReader();
    this.reader = reader;
    let configured = false;
    while (!this.stopped) {
      const { value, done } = await reader.read().catch(() => ({ value: undefined, done: true }));
      if (done || !value) break;
      try {
        if (!configured) {
          encoder.configure({
            codec: 'opus',
            sampleRate: value.sampleRate,
            numberOfChannels: value.numberOfChannels,
            bitrate: MICROPHONE_BITS,
          });
          configured = true;
        }
        first ??= { data: value.timestamp, perf: performance.now() };
        if (encoder.state === 'configured') encoder.encode(value);
      } finally {
        value.close();
      }
    }
  }
}

declare global {
  // Chrome's insertable streams for media (not yet in TypeScript's DOM types).
  class MediaStreamTrackProcessor<T = AudioData> {
    constructor(init: { track: MediaStreamTrack });
    readonly readable: ReadableStream<T>;
  }
}
