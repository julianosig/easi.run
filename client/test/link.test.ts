import assert from 'node:assert/strict';
import { test } from 'node:test';
import { linkedSession, wrongOrigin } from '../src/ui/api.ts';

test('a watch link names one session by its id', () => {
  assert.equal(linkedSession('#session=0123456789ab'), '0123456789ab');
  for (const hash of ['', '#', '#session=', '#session=0123456789AB', '#session=0123456789abc', '#session=../x', '#other=0123456789ab']) {
    assert.equal(linkedSession(hash), null, hash);
  }
});

test('a refusal for another address names the address the admin is set up for', () => {
  const refusal = (status: number, body: Record<string, unknown>) => Object.assign(new Error('refused'), { status, body });
  const origin = 'https://localhost:9443';
  assert.equal(wrongOrigin(refusal(403, { error: 'wrong address', reason: 'wrong-origin', origin })), origin);
  for (const e of [
    refusal(403, { error: 'forbidden' }),
    refusal(400, { reason: 'wrong-origin', origin }),
    refusal(403, { reason: 'other', origin }),
    refusal(403, { reason: 'wrong-origin', origin: 'javascript:alert(1)' }),
    refusal(403, { reason: 'wrong-origin', origin: 'http://localhost:9443' }),
    refusal(403, { reason: 'wrong-origin', origin: 'https://localhost:9443/path' }),
    refusal(403, { reason: 'wrong-origin', origin: 'https://user@evil.example' }),
    refusal(403, { reason: 'wrong-origin' }),
    new Error('network'),
  ]) {
    assert.equal(wrongOrigin(e), null, JSON.stringify(e));
  }
});
