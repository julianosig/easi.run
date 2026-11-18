import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { SessionInfo } from '../src/ui/api.ts';
import { forgetListed, known, lastListing, rememberListed, rememberListing } from '../src/ui/listed.ts';

const session = (id: string) => ({ id, url: 'https://a.example/' }) as SessionInfo;

test('a sidebar starts from what the last one read, until the login is over', () => {
  assert.equal(lastListing(), null);
  const read = { sessions: [session('aaaaaaaaaaaa'), session('bbbbbbbbbbbb')], profiles: { profiles: [], unlocked: true }, tokens: [], egresses: [] };
  rememberListing(read as unknown as Parameters<typeof rememberListing>[0]);
  assert.equal(lastListing()?.sessions.length, 2);
  // And a session opened from it starts from what it said of that session.
  assert.equal(known('bbbbbbbbbbbb')?.id, 'bbbbbbbbbbbb');
  // The sessions page's listing says what runs, and leaves the sidebar's as it was.
  rememberListed([session('cccccccccccc')]);
  assert.equal(known('cccccccccccc')?.id, 'cccccccccccc');
  assert.equal(lastListing()?.sessions.length, 2);
  forgetListed();
  assert.equal(lastListing(), null);
  assert.equal(known('cccccccccccc'), null);
});
