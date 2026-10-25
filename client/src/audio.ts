// The browser's sound: Opus packets of 20 ms (48 kHz stereo) decoded with WebCodecs and scheduled on an AudioContext
// a little ahead of time, so network jitter does not break the sound.

import { AUDIO_CHANNELS, AUDIO_FRAME_US, AUDIO_SAMPLE_RATE, type MessageOf } from './protocol.ts';

/** What the person hears: sound plays, waits for a click (the browser's autoplay rule), is muted, or cannot play. */
export type SoundState = 'playing' | 'blocked' | 'muted' | 'unsupported';

/** How far ahead of the audio clock a new stream starts, in seconds. */
const LEAD_S = 0.06;
/** Sound scheduled further ahead than this is late: the packet is dropped. */
const MAX_AHEAD_S = 0.3;
/** A packet older than this on arrival (on the server's clock) is stale and dropped, in microseconds. */
const MAX_AGE_US = 400_000;
/** Packets handed to the decoder and not yet out of it beyond this are dropped. */
const MAX_DECODE_QUEUE = 10;
/** How quickly a volume change takes effect, in seconds: a jump in gain would click. */
const VOLUME_RAMP_S = 0.015;

const CONFIG: AudioDecoderConfig = { codec: 'opus', sampleRate: AUDIO_SAMPLE_RATE, numberOfChannels: AUDIO_CHANNELS };

export class Sound {
  private context: AudioContext | null = null;
  /** Everything plays through this, at the person's volume. */
  private gain: GainNode | null = null;
  private decoder: AudioDecoder | null = null;
  /**
   * The timestamps of the packets in the decoder, oldest first. Its output is stamped on its own continuous timeline
   * from the first packet (Chrome's ignores later gaps), so each output is matched to its packet by order instead.
   */
  private decoding: number[] = [];
  private muted = false;
  private supported: boolean | null = null;
  /** Closed: nothing more is said, as a late answer (support, a resume, the context closing) would say "unsupported". */
  private closed = false;
  /** The next packet's expected timestamp, and the audio clock time it plays at. */
  private expectedUs: bigint | null = null;
  private playAt = 0;
  /** The server clock minus `performance.now()`, in microseconds, as of the last packet. */
  private offsetUs: number | undefined;
  private readonly sources = new Set<AudioBufferSourceNode>();
  private readonly resume = (): void => void this.context?.resume().then(() => this.notify(), () => this.notify());

  constructor(private readonly onState: (state: SoundState) => void) {
    if (typeof AudioDecoder === 'undefined' || typeof AudioContext === 'undefined') {
      this.supported = false;
      this.notify();
      return;
    }
    AudioDecoder.isConfigSupported(CONFIG)
      .then((support) => support.supported === true, () => false)
      .then((supported) => {
        this.supported = supported;
        this.notify();
      });
    // Created now, inside the click that opened the screen where possible; any later click or key resumes it.
    this.context = new AudioContext({ sampleRate: AUDIO_SAMPLE_RATE, latencyHint: 'interactive' });
    this.gain = this.context.createGain();
    this.gain.connect(this.context.destination);
    // A suspended context's clock stops, so what was scheduled would play late: drop it and start over on resume.
    this.context.onstatechange = () => {
      if (this.context?.state !== 'running') this.stop();
      this.notify();
    };
    document.addEventListener('pointerdown', this.resume, true);
    document.addEventListener('keydown', this.resume, true);
    this.notify();
  }

  state(): SoundState {
    if (this.supported === false || this.context === null) return 'unsupported';
    if (this.muted) return 'muted';
    return this.context.state === 'running' ? 'playing' : 'blocked';
  }

  private notify(): void {
    if (!this.closed) this.onState(this.state());
  }

  setMuted(muted: boolean): void {
    this.muted = muted;
    if (muted) this.stop();
    else this.resume();
    this.notify();
  }

  /** The volume here, from 0 (silent) to 1 (as the browser plays it); other values are ignored. */
  setVolume(volume: number): void {
    if (!Number.isFinite(volume) || this.context === null || this.gain === null) return;
    const level = Math.min(1, Math.max(0, volume));
    this.gain.gain.setTargetAtTime(level, this.context.currentTime, VOLUME_RAMP_S / 3);
  }

  /** One packet; `offsetUs` is the server clock minus `performance.now()` in microseconds, when known. */
  push(m: MessageOf<'audio'>, nowUs: number, offsetUs: number | undefined): void {
    const context = this.context;
    if (context === null || this.supported !== true || this.muted || context.state !== 'running') return;
    this.offsetUs = offsetUs;
    if (this.stale(Number(m.timestampUs), nowUs)) return;
    if (this.expectedUs !== m.timestampUs) {
      // A new stream, or packets were lost: it goes on after what is scheduled (cutting that would click), with the
      // same decoder, so nothing it holds is lost and it stays in step with the display's encoder, which goes on too.
      this.playAt = Math.max(this.playAt, context.currentTime + LEAD_S);
    }
    this.expectedUs = m.timestampUs + BigInt(AUDIO_FRAME_US);
    const decoder = this.decoder ?? this.startDecoder();
    if (this.decoding.length >= MAX_DECODE_QUEUE) return;
    this.decoding.push(Number(m.timestampUs));
    decoder.decode(new EncodedAudioChunk({ type: 'key', timestamp: Number(m.timestampUs), data: m.payload }));
  }

  private startDecoder(): AudioDecoder {
    const decoder: AudioDecoder = new AudioDecoder({
      // A decoder that was closed (muted, suspended) may still deliver: only the current one plays.
      output: (data) => {
        if (this.decoder !== decoder) {
          data.close();
          return;
        }
        this.play(data, this.decoding.shift());
      },
      error: () => {
        if (this.decoder !== decoder) return;
        this.decoder = null;
        this.decoding = [];
        this.expectedUs = null;
      },
    });
    decoder.configure(CONFIG);
    this.decoder = decoder;
    return decoder;
  }

  /** Whether sound stamped `timestampUs` on the server's clock is too old to play at `nowUs`. */
  private stale(timestampUs: number, nowUs: number): boolean {
    return this.offsetUs !== undefined && nowUs + this.offsetUs - timestampUs > MAX_AGE_US;
  }

  /** Plays decoded `data` of the packet stamped `timestampUs`. */
  private play(data: AudioData, timestampUs: number | undefined): void {
    const context = this.context;
    try {
      // Checked again here: decoding may have waited behind a stall.
      if (context === null || this.gain === null || this.muted || context.state !== 'running') return;
      if (timestampUs === undefined || this.stale(timestampUs, performance.now() * 1000)) return;
      const now = context.currentTime;
      // Behind the clock (an underrun): start again a little ahead. Too far ahead: drop, so delay stays bounded.
      if (this.playAt < now) this.playAt = now + LEAD_S;
      if (this.playAt > now + MAX_AHEAD_S) return;
      const buffer = context.createBuffer(data.numberOfChannels, data.numberOfFrames, data.sampleRate);
      // Straight into the buffer's own channels: no copy in between, nothing allocated per packet.
      for (let channel = 0; channel < data.numberOfChannels; channel++) {
        data.copyTo(buffer.getChannelData(channel), { planeIndex: channel, format: 'f32-planar' });
      }
      const source = context.createBufferSource();
      source.buffer = buffer;
      source.connect(this.gain);
      source.onended = () => this.sources.delete(source);
      source.start(this.playAt);
      this.sources.add(source);
      this.playAt += buffer.duration;
    } finally {
      data.close();
    }
  }

  /** Silences what is scheduled and forgets the stream. */
  private stop(): void {
    for (const source of this.sources) source.stop();
    this.sources.clear();
    if (this.decoder !== null && this.decoder.state !== 'closed') this.decoder.close();
    this.decoder = null;
    this.decoding = [];
    this.expectedUs = null;
    this.playAt = 0;
  }

  close(): void {
    this.closed = true;
    if (this.context !== null) this.context.onstatechange = null;
    this.stop();
    document.removeEventListener('pointerdown', this.resume, true);
    document.removeEventListener('keydown', this.resume, true);
    void this.context?.close();
    this.context = null;
    this.gain = null;
  }
}
