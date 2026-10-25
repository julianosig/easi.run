import assert from 'node:assert/strict';
import { test } from 'node:test';
import { checkPassword, MAX_PASSWORD, MIN_PASSWORD } from '../src/ui/setup.ts';

test('a new password needs the minimum, at most the maximum, and the same confirmation', () => {
  const long = 'correct horse';
  assert.deepEqual(checkPassword('', ''), { missing: MIN_PASSWORD, differs: false, problem: 'short' });
  assert.deepEqual(checkPassword('correct', 'correct'), { missing: 5, differs: false, problem: 'short' });
  assert.deepEqual(checkPassword(long, ''), { missing: 0, differs: false, problem: 'different' });
  assert.deepEqual(checkPassword(long, 'correct'), { missing: 0, differs: false, problem: 'different' }, 'still typing');
  assert.deepEqual(checkPassword(long, 'correct hors3'), { missing: 0, differs: true, problem: 'different' });
  assert.deepEqual(checkPassword(long, `${long}!`), { missing: 0, differs: true, problem: 'different' });
  assert.deepEqual(checkPassword(long, long), { missing: 0, differs: false, problem: null });
  // Counted in code points, as the admin counts them: twelve emoji are twelve characters.
  const emoji = '😀'.repeat(MIN_PASSWORD);
  assert.equal(checkPassword(emoji, emoji).problem, null);
  assert.equal(checkPassword(emoji.slice(2), emoji.slice(2)).missing, 1);
  const longest = 'a'.repeat(MAX_PASSWORD);
  assert.equal(checkPassword(longest, longest).problem, null);
  assert.equal(checkPassword(`${longest}a`, `${longest}a`).problem, 'long');
});
