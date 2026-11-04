import assert from 'node:assert/strict';
import { test } from 'node:test';
import { clampShare, clampTransparency, MAX_SHARE, parsePanel } from '../src/ui/chatPanel.ts';

test('the chat takes at most 70% of the window, in either place', () => {
  assert.equal(clampShare(0.95, 'bottom', 1000), MAX_SHARE);
  assert.equal(clampShare(0.95, 'side', 1600), MAX_SHARE);
  assert.equal(clampShare(0.4, 'side', 1600), 0.4);
});

test('the chat keeps room for its header and controls', () => {
  assert.equal(clampShare(0.01, 'bottom', 1000), 0.22);
  assert.equal(clampShare(0.01, 'side', 1600), 0.2625);
});

test('a remembered transparency is kept, even none at all', () => {
  assert.equal(parsePanel('{"transparency":0}').transparency, 0);
  assert.equal(parsePanel('{"transparency":55}').transparency, 55);
});

test('transparency stays between 0 and 70', () => {
  assert.equal(clampTransparency(-5), 0);
  assert.equal(clampTransparency(35.4), 35);
  assert.equal(clampTransparency(90), 70);
});

test('what cannot be read of a remembered layout falls back to the defaults', () => {
  const defaults = { view: 'bottom', shares: { bottom: 0.5, side: 0.4 }, transparency: 15 };
  assert.deepEqual(parsePanel(null), defaults);
  assert.deepEqual(parsePanel('not json'), defaults);
  assert.deepEqual(parsePanel('null'), defaults);
  assert.deepEqual(parsePanel('{"view":"top","shares":{"side":"x"},"transparency":"y"}'), defaults);
  assert.deepEqual(parsePanel('{"view":"side","shares":{"side":0.6},"transparency":200}'), {
    view: 'side',
    shares: { bottom: 0.5, side: 0.6 },
    transparency: 70,
  });
});
