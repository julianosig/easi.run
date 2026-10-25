import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { TokenInfo } from '../src/ui/api.ts';
import { allowedNames, choiceLabel, LOCAL, type ProfileInfo, profileSummary, toggled } from '../src/ui/profiles.ts';

const profile = (fields: Partial<ProfileInfo> = {}): ProfileInfo => ({
  id: '0123456789abcdef',
  name: 'Personal',
  allowed: [],
  created: 1,
  used: null,
  size: null,
  session: null,
  unsaved: null,
  ...fields,
});
const token = (id: string, name: string, revoked = false): TokenInfo => ({ id, name, created: 1, last_used: null, revoked });

test('a profile names who else may use it: the local clients, then each token still there', () => {
  const tokens = [token('aaaaaaaaaaaaaaaa', 'Laptop'), token('bbbbbbbbbbbbbbbb', 'Revoking', true)];
  assert.deepEqual(allowedNames(profile(), tokens), []);
  const allowed = ['bbbbbbbbbbbbbbbb', 'aaaaaaaaaaaaaaaa', LOCAL, 'cccccccccccccccc'];
  assert.deepEqual(allowedNames(profile({ allowed }), tokens), ['Local MCP', 'Laptop'], 'a revoked or gone token is not');
});

test('the allowed list takes and drops one entry, the local clients first', () => {
  assert.deepEqual(toggled([], 'aaaaaaaaaaaaaaaa', true), ['aaaaaaaaaaaaaaaa']);
  assert.deepEqual(toggled(['aaaaaaaaaaaaaaaa'], LOCAL, true), [LOCAL, 'aaaaaaaaaaaaaaaa']);
  assert.deepEqual(toggled([LOCAL, 'aaaaaaaaaaaaaaaa'], 'aaaaaaaaaaaaaaaa', true), [LOCAL, 'aaaaaaaaaaaaaaaa'], 'once');
  assert.deepEqual(toggled([LOCAL, 'aaaaaaaaaaaaaaaa'], LOCAL, false), ['aaaaaaaaaaaaaaaa']);
  assert.deepEqual(toggled([], LOCAL, false), []);
});

test('a profile says when it was saved, how large, and that a session uses it', () => {
  const when = (seconds: number) => `at ${seconds}`;
  assert.equal(profileSummary(profile(), when), 'Not saved yet');
  assert.equal(profileSummary(profile({ used: 5, size: 3 * 1024 * 1024 }), when), 'Saved at 5 · 3.0 MiB');
  assert.equal(profileSummary(profile({ used: 5, size: 10, session: '0123456789ab' }), when), 'Saved at 5 · 10 B · in use now');
  assert.equal(choiceLabel(profile()), 'Personal');
  assert.equal(choiceLabel(profile({ session: '0123456789ab' })), 'Personal (in use)');
});
