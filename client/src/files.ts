// Downloads from the browser: the display offers each one to the lease holder as soon as the browser starts it, and
// the admin application follows with a single-use ticket. The ticket goes to the person's own browser as a native
// download (its download manager shows the progress and writes the file to disk), so no byte of it passes through
// this page or its memory, whatever its size.

import type { MessageOf } from './protocol.ts';

/** One download as the person sees it. */
export interface Download {
  file: number;
  name: string;
  /** Its size, when the browser knows it (the person's download manager then shows percentage and time left). */
  size: number | null;
  state: 'starting' | 'downloading' | 'saved' | 'failed';
}

/** Downloads listed; the oldest finished ones go first, and an offer beyond it while none is finished is not taken. */
const MAX_LISTED = 32;
/** Offers and tickets kept for a lease this page has not heard of yet (they can arrive just before the lease message). */
const MAX_EARLY = 32;

interface Entry {
  download: Download;
  leaseEpoch: number;
}

type Early = MessageOf<'file-offer'> | MessageOf<'file-link'>;

/** Bytes as unpadded base64url. */
export function toBase64url(bytes: ArrayBuffer | Uint8Array): string {
  const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let binary = '';
  for (const byte of view) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** The download endpoint's address for a ticket. */
export function downloadUrl(session: string, ticket: Uint8Array): string {
  return `/api/sessions/${encodeURIComponent(session)}/files/${toBase64url(ticket)}`;
}

/** Hands `url` to the person's browser as a download, without leaving the page. */
function startNative(url: string): void {
  const link = document.createElement('a');
  link.href = url;
  link.download = '';
  link.rel = 'noopener';
  document.body.append(link);
  link.click();
  link.remove();
}

export class Downloads {
  private readonly entries: Entry[] = [];
  private early: Early[] = [];

  constructor(
    private readonly session: string,
    private readonly holds: (leaseEpoch: number) => boolean,
    private readonly changed: (downloads: Download[]) => void,
    private readonly fetch: (url: string) => void = startNative,
  ) {}

  list(): Download[] {
    return this.entries.map((e) => e.download);
  }

  /** A download the browser started, offered at `m.leaseEpoch`. */
  offer(m: MessageOf<'file-offer'>): void {
    if (this.keepEarly(m)) return;
    if (this.find(m.file, m.leaseEpoch) !== undefined) return;
    this.trim(MAX_LISTED - 1);
    if (this.entries.length >= MAX_LISTED) return;
    const size = m.size === null ? null : Number(m.size);
    this.entries.push({ download: { file: m.file, name: m.name, size, state: 'starting' }, leaseEpoch: m.leaseEpoch });
    this.changed(this.list());
  }

  /** The ticket of an offered download: the person's browser fetches it now. */
  link(m: MessageOf<'file-link'>): void {
    if (this.keepEarly(m)) return;
    const entry = this.find(m.file, m.leaseEpoch);
    if (entry?.download.state !== 'starting') return;
    entry.download.state = 'downloading';
    this.fetch(downloadUrl(this.session, m.ticket));
    this.changed(this.list());
  }

  /** A download written whole: its size is now known (a recording's never is before). */
  end(m: MessageOf<'file-end'>): void {
    this.settle(m.file, m.leaseEpoch, 'saved', Number(m.size));
  }

  gone(m: MessageOf<'file-gone'>): void {
    this.settle(m.file, m.leaseEpoch, 'failed');
  }

  /** This connection now holds control at `leaseEpoch`: what arrived early for it counts now. */
  leaseHeld(leaseEpoch: number): void {
    const early = this.early.filter((m) => m.leaseEpoch === leaseEpoch);
    this.early = [];
    for (const m of early) {
      if (m.type === 'file-offer') this.offer(m);
      else this.link(m);
    }
  }

  /**
   * The lease moved (to `next`, whose early messages are kept) or the stream ended: downloads of the previous lease
   * end (the admin application ends their responses too).
   */
  stop(next?: number): void {
    this.early = this.early.filter((m) => m.leaseEpoch === next);
    for (const e of this.entries) {
      if (e.download.state === 'starting' || e.download.state === 'downloading') e.download.state = 'failed';
    }
    this.changed(this.list());
  }

  /** Possibly for a lease announced right after it: kept briefly, never acted on unless that lease is ours. */
  private keepEarly(m: Early): boolean {
    if (this.holds(m.leaseEpoch)) return false;
    this.early = [...this.early, m].slice(-MAX_EARLY);
    return true;
  }

  private find(file: number, leaseEpoch: number): Entry | undefined {
    return this.entries.find((e) => e.download.file === file && e.leaseEpoch === leaseEpoch);
  }

  private settle(file: number, leaseEpoch: number, state: 'saved' | 'failed', size?: number): void {
    const entry = this.find(file, leaseEpoch);
    if (entry === undefined || entry.download.state === 'saved' || entry.download.state === 'failed') return;
    entry.download.state = state;
    if (size !== undefined) entry.download.size = size;
    this.changed(this.list());
  }

  /** Drops finished entries, oldest first, until at most `keep` remain or none is finished. */
  private trim(keep: number): void {
    while (this.entries.length > keep) {
      const done = this.entries.findIndex((e) => e.download.state === 'saved' || e.download.state === 'failed');
      if (done < 0) break;
      this.entries.splice(done, 1);
    }
  }
}
