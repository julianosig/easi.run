// WebCodecs video decoding for the stream's video region.

import { AV1_CODEC_STRING, CODEC_AV1, CODEC_H264, Codec, H264_CODEC_STRING } from './protocol.ts';

/**
 * The decoder configuration of each codec. H.264 is an Annex B byte stream carrying SPS and PPS in every keyframe, so
 * it has no `description`.
 */
function decoderConfig(codec: Codec): VideoDecoderConfig {
  switch (codec) {
    case Codec.H264:
      return { codec: H264_CODEC_STRING, optimizeForLatency: true, hardwareAcceleration: 'no-preference' };
    case Codec.Av1:
      return { codec: AV1_CODEC_STRING, optimizeForLatency: true };
    case Codec.None:
      throw new Error('no video codec');
  }
}

export interface Probe {
  /** The hello codec bit set. */
  codecs: number;
  /** Per codec: whether a hardware decoder is available, or null when the browser does not say. */
  hardware: Partial<Record<Codec, boolean | null>>;
}

async function supported(config: VideoDecoderConfig): Promise<boolean | null> {
  try {
    return (await VideoDecoder.isConfigSupported(config)).supported === true;
  } catch {
    return null;
  }
}

/**
 * Probes WebCodecs for every codec of the protocol. A codec counts when its configuration is supported; its hardware
 * hint is whether the same configuration is supported with `prefer-hardware`.
 */
export async function probeCodecs(): Promise<Probe> {
  const probe: Probe = { codecs: 0, hardware: {} };
  if (typeof VideoDecoder === 'undefined') return probe;
  const candidates = [
    [Codec.Av1, CODEC_AV1],
    [Codec.H264, CODEC_H264],
  ] as const;
  await Promise.all(
    candidates.map(async ([codec, bit]) => {
      const config = decoderConfig(codec);
      if ((await supported(config)) !== true) return;
      probe.codecs |= bit;
      probe.hardware[codec] = await supported({ ...config, hardwareAcceleration: 'prefer-hardware' });
    }),
  );
  return probe;
}

interface VideoHandlers {
  /** A decoded frame; the handler owns it and must close it. */
  output(frame: VideoFrame, seq: number): void;
  /** The decoder failed and was closed; the next keyframe starts a new one. */
  error(error: Error): void;
}

/** Decode-time samples kept per statistics interval. */
const MAX_SAMPLES = 1024;
/**
 * A decoder counts as stuck when a chunk has waited in it longer than `STUCK_MS` with no frame out of it for as long,
 * or longer than `MAX_RESIDENCE_MS` whatever comes out (it cannot keep up). A burst of chunks it clears in well under a
 * second is no reason to drop it: a reset costs a keyframe's round trip with nothing drawn meanwhile.
 */
export const STUCK_MS = 1000;
export const MAX_RESIDENCE_MS = 2000;

/** One codec's decoder. Each chunk's timestamp is the ordering sequence number, which comes back on its frame. */
export class VideoStream {
  private decoder: VideoDecoder | null = null;
  private readonly submitted = new Map<number, number>();
  private samples: number[] = [];
  /** When a frame last came out, or a chunk went in with none waiting (`performance.now`). */
  private progressAt = 0;

  constructor(
    readonly codec: Codec,
    private readonly handlers: VideoHandlers,
  ) {}

  /** Whether the decoder is stuck at `now` (see `STUCK_MS`). */
  stuck(now: number): boolean {
    const oldest = this.submitted.values().next();
    if (oldest.done === true) return false;
    const waited = now - oldest.value;
    return waited > MAX_RESIDENCE_MS || (waited > STUCK_MS && now - this.progressAt > STUCK_MS);
  }

  /** Queues one chunk. Without a live decoder only a keyframe starts one; a refused chunk reports an error. */
  decode(payload: Uint8Array, keyframe: boolean, seq: number): void {
    if (this.decoder === null) {
      if (!keyframe) return;
      this.decoder = this.open();
    }
    try {
      const now = performance.now();
      // A chunk after an idle decoder starts the wait for output afresh.
      if (this.submitted.size === 0) this.progressAt = now;
      this.submitted.set(seq, now);
      this.decoder.decode(new EncodedVideoChunk({ type: keyframe ? 'key' : 'delta', timestamp: seq, data: payload }));
    } catch (e) {
      this.fail(e);
    }
  }

  private open(): VideoDecoder {
    const decoder: VideoDecoder = new VideoDecoder({
      output: (frame) => {
        if (this.decoder !== decoder) {
          frame.close();
          return;
        }
        const seq = frame.timestamp;
        const start = this.submitted.get(seq);
        this.submitted.delete(seq);
        this.progressAt = performance.now();
        if (start !== undefined && this.samples.length < MAX_SAMPLES) {
          this.samples.push(Math.round((performance.now() - start) * 1000));
        }
        this.handlers.output(frame, seq);
      },
      error: (e) => {
        if (this.decoder === decoder) this.fail(e);
      },
    });
    decoder.configure(decoderConfig(this.codec));
    return decoder;
  }

  private fail(e: unknown): void {
    this.reset();
    this.handlers.error(e instanceof Error ? e : new Error(String(e)));
  }

  /** The median decode time of the frames output since the last call, in microseconds (0 without frames). */
  takeDecodeP50(): number {
    const sorted = this.samples.sort((a, b) => a - b);
    this.samples = [];
    return sorted.length === 0 ? 0 : sorted[Math.floor(sorted.length / 2)]!;
  }

  /** Drops the decoder and its queue; the next keyframe starts a new one. */
  reset(): void {
    const decoder = this.decoder;
    this.decoder = null;
    this.submitted.clear();
    if (decoder !== null && decoder.state !== 'closed') decoder.close();
  }
}
