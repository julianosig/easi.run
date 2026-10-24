import assert from 'node:assert/strict';
import { test } from 'node:test';
import { linkedSession } from '../src/ui/api.ts';

test('a watch link names one session by its id', () => {
  assert.equal(linkedSession('#session=0123456789ab'), '0123456789ab');
  for (const hash of ['', '#', '#session=', '#session=0123456789AB', '#session=0123456789abc', '#session=../x', '#other=0123456789ab']) {
    assert.equal(linkedSession(hash), null, hash);
  }
});
