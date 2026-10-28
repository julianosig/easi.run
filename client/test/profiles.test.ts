import assert from 'node:assert/strict';
import { test } from 'node:test';
import sessions from '../messages/en/sessions.json';
import { format, type Message, type Params } from '../../i18n/format.ts';
import { formatSize, type TokenInfo } from '../src/ui/api.ts';
import { allowedNames, LOCAL, type ProfileInfo, profileSummary, type Translate, toggled } from '../src/ui/profiles.ts';

/** The page's English, as `t` gives it. */
const t: Translate = (key, ...[params]) =>
  format((sessions as Record<string, Message>)[key.slice('sessions.'.length)] ?? key, (params ?? {}) as Params, 'en');

const profile = (fields: Partial<ProfileInfo> = {}): ProfileInfo => ({
  id: '0123456789abcdef',
  name: 'Personal',
  allowed: [],
  created: 1,
  used: null,
  size: null,
  session: null,
  unsaved: null,
  egress: null,
  ...fields,
});
const token = (id: string, name: string, revoked = false): TokenInfo => ({ id, name, created: 1, last_used: null, revoked });

test('a profile names who else may use it: the local clients, then each token still there', () => {
  const tokens = [token('aaaaaaaaaaaaaaaa', 'Laptop'), token('bbbbbbbbbbbbbbbb', 'Revoking', true)];
  assert.deepEqual(allowedNames(profile(), tokens, t), []);
  const allowed = ['bbbbbbbbbbbbbbbb', 'aaaaaaaaaaaaaaaa', LOCAL, 'cccccccccccccccc'];
  assert.deepEqual(allowedNames(profile({ allowed }), tokens, t), ['Agents on this server', 'Laptop'], 'a revoked or gone token is not');
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
  assert.equal(profileSummary(profile(), when, t, 'en'), 'Not saved yet');
  assert.equal(profileSummary(profile({ used: 5, size: 3_100_000 }), when, t, 'en'), 'Saved at 5 · 3.1 MB');
  assert.equal(profileSummary(profile({ used: 5, size: 10, session: '0123456789ab' }), when, t, 'en'), 'Saved at 5 · 10 byte · in use now');
});

test('a size reads in the page’s language, its unit as that language writes it', () => {
  assert.equal(formatSize(999, 'en'), '999 byte');
  assert.equal(formatSize(7_300_000, 'en'), '7.3 MB');
  assert.equal(formatSize(2_500_000_000, 'en'), '2.5 GB');
  assert.equal(/^7,3\s+Mo$/u.test(formatSize(7_300_000, 'fr')), true, 'French writes Mo');
});
