import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { Message } from '../src/protocol.ts';
import { Uploads } from '../src/uploads.ts';

const settle = () => new Promise((r) => setTimeout(r, 0));

function setup() {
  const sent: Message[] = [];
  const uploads = new Uploads(
    (m) => sent.push(m),
    (e) => e === 3,
    () => {},
  );
  return { uploads, sent };
}

const starts = (sent: Message[]) => sent.filter((m) => m.type === 'upload-start').map((m) => m.type === 'upload-start' && m.name);

test('a second batch waits for the first instead of abandoning it', async () => {
  const { uploads, sent } = setup();
  const first = uploads.upload([new File([new Uint8Array(10)], 'a.txt')], 3);
  const second = uploads.upload([new File([], 'b.txt')], 3);
  await settle();
  assert.deepEqual(starts(sent), ['a.txt']);
  uploads.ack({ type: 'upload-ack', leaseEpoch: 3, upload: 1, received: 10n });
  uploads.done({ type: 'upload-done', leaseEpoch: 3, upload: 1, stored: true });
  assert.deepEqual(await first, [1]);
  await settle();
  // The empty file needs no data: its start, then the result.
  assert.deepEqual(starts(sent), ['a.txt', 'b.txt']);
  uploads.done({ type: 'upload-done', leaseEpoch: 3, upload: 2, stored: true });
  assert.deepEqual(await second, [2]);
  assert.equal(sent.filter((m) => m.type === 'upload-data').length, 1);
});

test('a lease change fails the upload in progress', async () => {
  const { uploads } = setup();
  const batch = uploads.upload([new File([new Uint8Array(10)], 'a.txt')], 3);
  await settle();
  uploads.stop();
  assert.deepEqual(await batch, []);
});

test('a batch reports the bytes the server received, across its files, then its end', async () => {
  const { uploads } = setup();
  const seen: string[] = [];
  const batch = uploads.upload(
    [new File([new Uint8Array(10)], 'a.txt'), new File([new Uint8Array(30)], 'b.txt')],
    3,
    (p) => seen.push(`${p.name}:${p.files}:${p.received}/${p.total}:${p.state}`),
  );
  await settle();
  uploads.ack({ type: 'upload-ack', leaseEpoch: 3, upload: 1, received: 10n });
  uploads.done({ type: 'upload-done', leaseEpoch: 3, upload: 1, stored: true });
  await settle();
  uploads.ack({ type: 'upload-ack', leaseEpoch: 3, upload: 2, received: 15n });
  uploads.ack({ type: 'upload-ack', leaseEpoch: 3, upload: 2, received: 30n });
  uploads.done({ type: 'upload-done', leaseEpoch: 3, upload: 2, stored: true });
  assert.deepEqual(await batch, [1, 2]);
  assert.deepEqual(seen, [
    'a.txt:2:0/40:sending',
    'a.txt:2:10/40:sending',
    'a.txt:2:10/40:sending',
    'a.txt:2:25/40:sending',
    'a.txt:2:40/40:finishing',
    'a.txt:2:40/40:finishing',
    'a.txt:2:40/40:done',
  ]);
});

test('a batch whose upload fails ends failed', async () => {
  const { uploads } = setup();
  let last = '';
  const batch = uploads.upload([new File([new Uint8Array(10)], 'a.txt')], 3, (p) => (last = p.state));
  await settle();
  uploads.stop();
  assert.deepEqual(await batch, []);
  assert.equal(last, 'failed');
});
