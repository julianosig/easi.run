import assert from 'node:assert/strict';
import { test } from 'node:test';
import { type Download, Downloads, downloadUrl } from '../src/files.ts';
import type { MessageOf } from '../src/protocol.ts';

function setup(epoch = 3) {
  const fetched: string[] = [];
  let held = epoch;
  let list: Download[] = [];
  const downloads = new Downloads(
    'sess',
    (e) => e === held,
    (l) => (list = l),
    (url) => fetched.push(url),
  );
  return { downloads, fetched, take: (e: number) => (held = e), list: () => list.map((d) => `${d.name}:${d.state}`) };
}

const ticket = (n: number): Uint8Array => new Uint8Array(32).fill(n);
const offer = (file: number, leaseEpoch = 3): MessageOf<'file-offer'> => ({
  type: 'file-offer',
  leaseEpoch,
  file,
  size: file === 1 ? 5_000_000_000n : null,
  name: `f${file}`,
});
const link = (file: number, leaseEpoch = 3): MessageOf<'file-link'> => ({
  type: 'file-link',
  leaseEpoch,
  file,
  ticket: ticket(file),
});

test('a ticket becomes the download endpoint address, URL-safe and unpadded', () => {
  const t = Uint8Array.from({ length: 32 }, (_, i) => [0xfb, 0xff, 0xbf][i % 3] ?? 0);
  const url = downloadUrl('s 1', t);
  assert.equal(url, `/api/sessions/s%201/files/${'-_'.repeat(20)}-_8`);
});

test('an offered download is handed to the browser once, when its ticket comes, and ends as the server says', () => {
  const { downloads, fetched, list } = setup();
  downloads.offer(offer(1));
  assert.deepEqual(list(), ['f1:starting']);
  assert.equal(downloads.list()[0]?.size, 5_000_000_000);
  assert.deepEqual(fetched, []);
  downloads.link(link(1));
  downloads.link(link(1));
  assert.deepEqual(fetched, [downloadUrl('sess', ticket(1))]);
  assert.deepEqual(list(), ['f1:downloading']);
  downloads.end({ type: 'file-end', leaseEpoch: 3, file: 1, size: 10n });
  assert.deepEqual(list(), ['f1:saved']);
  assert.equal(downloads.list()[0]?.size, 10, 'the size it ended with');
  downloads.offer(offer(2));
  downloads.link(link(2));
  downloads.gone({ type: 'file-gone', leaseEpoch: 3, file: 2 });
  assert.deepEqual(list(), ['f1:saved', 'f2:failed']);
});

test('a ticket without its offer, or of another lease, fetches nothing', () => {
  const { downloads, fetched } = setup();
  downloads.link(link(9));
  downloads.offer(offer(1));
  downloads.link(link(1, 4));
  assert.deepEqual(fetched, []);
});

test('an offer and ticket that arrive before their lease are acted on once that lease is ours', () => {
  const { downloads, fetched, take, list } = setup(3);
  downloads.offer(offer(1, 4));
  downloads.link(link(1, 4));
  assert.deepEqual(fetched, []);
  take(4);
  downloads.stop(4);
  downloads.leaseHeld(4);
  assert.deepEqual(fetched, [downloadUrl('sess', ticket(1))]);
  assert.deepEqual(list(), ['f1:downloading']);
});

test('a lease change fails the downloads in progress and drops early messages of other leases', () => {
  const { downloads, fetched, take, list } = setup(3);
  downloads.offer(offer(1));
  downloads.link(link(1));
  downloads.offer(offer(2, 9));
  take(5);
  downloads.stop(5);
  downloads.leaseHeld(5);
  take(9);
  downloads.leaseHeld(9);
  assert.deepEqual(list(), ['f1:failed']);
  assert.equal(fetched.length, 1);
});

test('after 32 finished downloads, a new one still makes room', () => {
  const { downloads, list } = setup();
  for (let file = 1; file <= 32; file++) {
    downloads.offer(offer(file));
    downloads.gone({ type: 'file-gone', leaseEpoch: 3, file });
  }
  downloads.offer(offer(33));
  assert.equal(list().length, 32);
  assert.equal(list().at(-1), 'f33:starting');
});
