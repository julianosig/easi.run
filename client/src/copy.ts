// What the page copied, offered to the person holding control: written to their clipboard at once where the browser
// lets a page write it (the viewer's auto-copy, which the admin page turns on), otherwise kept for a click. Every copy
// is fenced by the lease it arrived under and by a generation, so a later copy, a lease change, or the end of the
// stream withdraws its offer; a clipboard write already handed to the browser cannot be taken back, and its answer
// then changes nothing.

export type CopyOffer = { text: string; written: boolean } | null;

export class Copies {
  private offered: { text: string; leaseEpoch: number } | null = null;
  private generation = 0;

  constructor(
    private readonly holds: (leaseEpoch: number) => boolean,
    private readonly write: (text: string) => Promise<void>,
    private readonly changed: (offer: CopyOffer) => void,
  ) {}

  /** A copy that arrived at `leaseEpoch`; `auto` writes it at once (the viewer's auto-copy is on). */
  arrived(text: string, leaseEpoch: number, auto: boolean): void {
    if (!this.holds(leaseEpoch)) return;
    const generation = ++this.generation;
    this.offered = { text, leaseEpoch };
    const offer = (written: boolean): void => {
      if (generation === this.generation && this.holds(leaseEpoch)) this.changed({ text, written });
    };
    if (!auto) return offer(false);
    this.write(text).then(
      () => offer(true),
      () => offer(false),
    );
  }

  /** The person clicked to copy the offer: refused once it was withdrawn; reported written only if still current. */
  writeOffered(): Promise<void> {
    const copy = this.offered;
    if (copy === null || !this.holds(copy.leaseEpoch)) return Promise.reject(new Error('nothing to copy'));
    const generation = this.generation;
    return this.write(copy.text).then(() => {
      if (generation === this.generation && this.holds(copy.leaseEpoch)) this.changed({ text: copy.text, written: true });
    });
  }

  /** Control changed hands or the stream ended. */
  withdraw(): void {
    this.generation++;
    if (this.offered !== null) this.changed(null);
    this.offered = null;
  }
}
