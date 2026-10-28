import assert from 'node:assert/strict';
import { test } from 'node:test';
import { linkedSession, toldBy, wrongOrigin } from '../src/ui/api.ts';

test('a watch link names one session by its id', () => {
  assert.equal(linkedSession('#session=0123456789ab'), '0123456789ab');
  for (const hash of ['', '#', '#session=', '#session=0123456789AB', '#session=0123456789abc', '#session=../x', '#other=0123456789ab']) {
    assert.equal(linkedSession(hash), null, hash);
  }
});

test('a refusal for another address names the address the admin is set up for', () => {
  const refusal = (status: number, body: Record<string, unknown>) => Object.assign(new Error('refused'), { status, body });
  const origin = 'https://localhost:9443';
  assert.equal(wrongOrigin(refusal(403, { error: 'wrong address', code: 'wrong-origin', origin })), origin);
  for (const e of [
    refusal(403, { error: 'forbidden' }),
    refusal(400, { code: 'wrong-origin', origin }),
    refusal(403, { code: 'other', origin }),
    refusal(403, { code: 'wrong-origin', origin: 'javascript:alert(1)' }),
    refusal(403, { code: 'wrong-origin', origin: 'http://localhost:9443' }),
    refusal(403, { code: 'wrong-origin', origin: 'https://localhost:9443/path' }),
    refusal(403, { code: 'wrong-origin', origin: 'https://user@evil.example' }),
    refusal(403, { code: 'wrong-origin' }),
    new Error('network'),
  ]) {
    assert.equal(wrongOrigin(e), null, JSON.stringify(e));
  }
});

test('a refusal is told by its code and its values, never by the admin\'s English', () => {
  const told = toldBy({ code: 'too-many-tokens', error: 'at most 32 tokens; revoke one first', max: 32, nested: { a: 1 } }, 409);
  assert.deepEqual(told, { key: 'errors.too-many-tokens', params: { max: 32 }, fallback: 'errors.generic.refused' });
  // No code, or one that is no message's name: said by its status alone.
  assert.equal(toldBy({ error: 'no such session' }, 404).key, null);
  for (const code of ['', 'Wrong', '../x', 'a--b', '-a', 'a-', 7]) assert.equal(toldBy({ code }, 400).key, null, String(code));
  assert.equal(toldBy({}, 502).fallback, 'errors.generic.failed');
  assert.equal(toldBy({ code: 'save-failed' }, undefined).fallback, 'errors.generic.failed', 'a failure inside an answer');
  assert.equal(toldBy(null, undefined, true).fallback, 'errors.generic.network', 'a request that got no answer');
});
