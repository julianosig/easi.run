import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Copies, type CopyOffer } from '../src/copy.ts';

function setup(auto: boolean) {
  let epoch = 3;
  const offers: CopyOffer[] = [];
  const pending: (() => void)[] = [];
  const written: string[] = [];
  const copies = new Copies(
    (leaseEpoch) => leaseEpoch === epoch,
    (text) =>
      new Promise<void>((resolve) => {
        written.push(text);
        pending.push(resolve);
      }),
    (offer) => offers.push(offer),
  );
  return { copies, offers, written, pending, auto, revoke: () => (epoch += 1) };
}

const settle = () => new Promise((r) => setTimeout(r, 0));

test('a copy written after control changed hands is never reported, and the offer is withdrawn', async () => {
  const s = setup(true);
  s.copies.arrived('secret', 3, true);
  s.revoke();
  s.copies.withdraw();
  s.pending[0]!();
  await settle();
  assert.deepEqual(s.offers, [null]);
  let refused = false;
  await s.copies.writeOffered().catch(() => (refused = true));
  assert(refused, 'a withdrawn offer cannot be written');
});

test('a later copy supersedes a write still in flight', async () => {
  const s = setup(true);
  s.copies.arrived('first', 3, true);
  s.copies.arrived('second', 3, true);
  s.pending[0]!();
  s.pending[1]!();
  await settle();
  assert.deepEqual(s.offers, [{ text: 'second', written: true }]);
});

test('without automatic copying the offer waits for the click, which writes the latest copy of this lease', async () => {
  const s = setup(false);
  s.copies.arrived('one', 3, false);
  assert.deepEqual(s.offers, [{ text: 'one', written: false }]);
  assert.deepEqual(s.written, []);
  const click = s.copies.writeOffered();
  s.pending[0]!();
  await click;
  assert.deepEqual(s.written, ['one']);
  assert.deepEqual(s.offers.at(-1), { text: 'one', written: true });
  // A copy fenced by another lease is ignored.
  s.copies.arrived('stale', 2, false);
  assert.equal(s.offers.length, 2);
});

test('a click on an older copy does not hide a newer offer', async () => {
  const s = setup(false);
  s.copies.arrived('a', 3, false);
  const click = s.copies.writeOffered();
  s.copies.arrived('b', 3, false);
  s.pending[0]!();
  await click;
  assert.deepEqual(s.offers, [
    { text: 'a', written: false },
    { text: 'b', written: false },
  ]);
});
