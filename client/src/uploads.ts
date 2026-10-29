// Files the person gives the page: chosen in their own file picker when the page opens its file chooser, or dragged onto
// the screen. Each file goes in order, in pieces, never more than the protocol's window ahead of the server's last
// acknowledgement, one file at a time; the server stores it in the browser and says so.

import { textChunks } from './input.ts';
import { type ClientMessage, MAX_CHOSEN, MAX_FILE_NAME, type MessageOf, UPLOAD_CHUNK, UPLOAD_WINDOW } from './protocol.ts';

/** One upload as the person sees it. */
export interface Upload {
  upload: number;
  name: string;
  size: number;
  sent: number;
  state: 'sending' | 'stored' | 'failed';
}

/**
 * One batch as the person sees it (a drop's files, or a chooser's): the bytes the server acknowledged out of all its
 * files', until every file is stored (`done`) or one is not (`failed`). `finishing`: everything was received, and the
 * server has yet to say the last file is stored.
 */
export interface BatchProgress {
  /** The first file's name. */
  name: string;
  files: number;
  /** Files beyond what one batch takes, left out. */
  left: number;
  total: number;
  received: number;
  state: 'sending' | 'finishing' | 'done' | 'failed';
}

/** Largest file the server takes. */
const MAX_UPLOAD = 512 * 1024 * 1024;
/** Uploads listed; the oldest go first. */
const MAX_LISTED = 32;
/** An upload with no answer from the server for this long has failed. */
const STALL_MS = 15_000;

interface Transfer {
  upload: Upload;
  file: File;
  leaseEpoch: number;
  acked: number;
  reading: boolean;
  settle: (stored: boolean) => void;
  /** Called on each acknowledgement, for its batch's progress. */
  acknowledged?: () => void;
}

/** A file name the protocol carries: at most MAX_FILE_NAME bytes of UTF-8, never empty. */
function protocolName(name: string): string {
  return textChunks(name, MAX_FILE_NAME)[0] ?? 'upload';
}

export class Uploads {
  private readonly listed: Upload[] = [];
  private current: Transfer | null = null;
  private next = 0;
  /** Batches run one after another: a drop while a chooser's files are still going waits its turn. */
  private queue: Promise<unknown> = Promise.resolve();
  private stall: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private readonly send: (m: ClientMessage) => void,
    private readonly holds: (leaseEpoch: number) => boolean,
    private readonly changed: (uploads: Upload[]) => void,
  ) {}

  /**
   * Sends `files` one after another at `leaseEpoch`, after any batch before; resolves with the ids the browser stored.
   * `progress`, if given, hears the batch's own progress from now until it ends.
   */
  upload(files: File[], leaseEpoch: number, progress?: (p: BatchProgress) => void): Promise<number[]> {
    const taken = files.slice(0, MAX_CHOSEN);
    const total = taken.reduce((n, f) => n + f.size, 0);
    let settledBytes = 0;
    let current: Transfer | null = null;
    const report = (state: BatchProgress['state']): void => {
      const received = Math.min(total, settledBytes + (current?.acked ?? 0));
      const shown = state === 'sending' && received >= total && total > 0 ? 'finishing' : state;
      progress?.({ name: taken[0]?.name ?? '', files: taken.length, left: files.length - taken.length, total, received, state: shown });
    };
    report('sending');
    const batch = this.queue.then(async () => {
      const stored: number[] = [];
      for (const file of taken) {
        if (!this.holds(leaseEpoch)) break;
        const id = await this.one(file, leaseEpoch, (t) => {
          current = t;
          t.acknowledged = () => report('sending');
        });
        // Only bytes the server received count; a file that failed adds what it got that far, and the batch goes on.
        settledBytes += id !== null ? file.size : (current?.acked ?? 0);
        current = null;
        if (id !== null) stored.push(id);
        report('sending');
      }
      report(stored.length === taken.length ? 'done' : 'failed');
      return stored;
    });
    this.queue = batch.catch(() => undefined);
    return batch;
  }

  private one(file: File, leaseEpoch: number, started?: (t: Transfer) => void): Promise<number | null> {
    this.next += 1;
    const upload: Upload = { upload: this.next, name: protocolName(file.name), size: file.size, sent: 0, state: 'sending' };
    this.listed.push(upload);
    while (this.listed.length > MAX_LISTED) this.listed.shift();
    return new Promise((resolve) => {
      const settle = (stored: boolean): void => {
        if (upload.state !== 'sending') return;
        upload.state = stored ? 'stored' : 'failed';
        if (this.current?.upload === upload) {
          this.current = null;
          this.unwatch();
        }
        this.changed([...this.listed]);
        resolve(stored ? upload.upload : null);
      };
      if (file.size > MAX_UPLOAD) return settle(false);
      this.current = { upload, file, leaseEpoch, acked: 0, reading: false, settle };
      started?.(this.current);
      this.watch();
      this.send({ type: 'upload-start', leaseEpoch, upload: upload.upload, size: BigInt(file.size), name: upload.name });
      this.changed([...this.listed]);
      void this.pump();
    });
  }

  /** Sends pieces while the window allows. */
  private async pump(): Promise<void> {
    const t = this.current;
    if (t === null || t.reading) return;
    t.reading = true;
    try {
      while (this.current === t && t.upload.sent < t.upload.size && t.upload.sent - t.acked < UPLOAD_WINDOW) {
        if (!this.holds(t.leaseEpoch)) return t.settle(false);
        const end = Math.min(t.upload.sent + UPLOAD_CHUNK, t.upload.size, t.acked + UPLOAD_WINDOW);
        const data = new Uint8Array(await t.file.slice(t.upload.sent, end).arrayBuffer());
        if (this.current !== t) return;
        this.send({ type: 'upload-data', leaseEpoch: t.leaseEpoch, upload: t.upload.upload, offset: BigInt(t.upload.sent), data });
        t.upload.sent = end;
      }
      this.changed([...this.listed]);
    } catch {
      t.settle(false);
    } finally {
      t.reading = false;
    }
  }

  ack(m: MessageOf<'upload-ack'>): void {
    const t = this.current;
    if (t === null || t.upload.upload !== m.upload || t.leaseEpoch !== m.leaseEpoch) return;
    t.acked = Math.max(t.acked, Number(m.received));
    t.acknowledged?.();
    this.watch();
    void this.pump();
  }

  done(m: MessageOf<'upload-done'>): void {
    const t = this.current;
    if (t === null || t.upload.upload !== m.upload || t.leaseEpoch !== m.leaseEpoch) return;
    t.settle(m.stored && t.upload.sent === t.upload.size);
  }

  /** The lease moved or the stream ended: the upload in progress fails (the server abandons it too). */
  stop(): void {
    this.current?.settle(false);
    this.current = null;
    this.unwatch();
  }

  /** Fails the upload in progress if the server answers nothing for a while (an answer may have been lost). */
  private watch(): void {
    this.unwatch();
    this.stall = setTimeout(() => this.current?.settle(false), STALL_MS);
  }

  private unwatch(): void {
    if (this.stall !== null) clearTimeout(this.stall);
    this.stall = null;
  }
}
