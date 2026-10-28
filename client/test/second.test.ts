import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  appCode,
  assertionJSON,
  creationOptions,
  fromBase64url,
  passkeyRefusal,
  recoveryFile,
  registrationJSON,
  requestOptions,
} from '../src/ui/second.ts';
import { toBase64url } from '../src/files.ts';

const bytes = (...values: number[]) => new Uint8Array(values);

test('base64url carries every byte both ways, without padding', () => {
  for (const length of [0, 1, 2, 3, 4, 31, 32, 33]) {
    const original = Uint8Array.from({ length }, (_, i) => (i * 97 + 251) % 256);
    const text = toBase64url(original);
    assert(/^[A-Za-z0-9_-]*$/.test(text), text);
    assert.deepEqual([...fromBase64url(text)], [...original], `${length} bytes`);
  }
  assert.equal(toBase64url(bytes(0xfb, 0xff, 0xbf)), '-_-_');
  assert.equal(toBase64url(bytes(0xfb, 0xff, 0xbf).buffer), '-_-_', 'an ArrayBuffer too');
});

test('an app code is its six digits, spaces between them left out', () => {
  for (const [typed, code] of [
    ['123456', '123456'],
    [' 123 456 ', '123456'],
    ['123-456', '123456'],
  ] as const) {
    assert.equal(appCode(typed), code, typed);
  }
  for (const typed of ['', '12345', '1234567', '12345a', 'abcd-efgh-jkmn-pqrs']) assert.equal(appCode(typed), null, typed);
});

test('the options the admin answers become what WebAuthn takes, binary fields as bytes', () => {
  const created = creationOptions({
    publicKey: {
      rp: { id: 'localhost', name: 'easi.run' },
      user: { id: toBase64url(bytes(1, 2, 3)), name: 'admin', displayName: 'easi.run admin' },
      challenge: toBase64url(bytes(9, 8, 7)),
      pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
      excludeCredentials: [{ type: 'public-key', id: toBase64url(bytes(5, 5)) }],
      authenticatorSelection: { userVerification: 'required' },
    },
  });
  assert.deepEqual([...(created.challenge as Uint8Array)], [9, 8, 7]);
  assert.deepEqual([...(created.user.id as Uint8Array)], [1, 2, 3]);
  assert.equal(created.user.name, 'admin');
  assert.deepEqual([...(created.excludeCredentials?.[0]?.id as Uint8Array)], [5, 5]);
  assert.equal(created.authenticatorSelection?.userVerification, 'required', 'the rest as it came');
  const requested = requestOptions({
    publicKey: { challenge: toBase64url(bytes(4)), rpId: 'localhost', userVerification: 'required' },
  });
  assert.deepEqual([...(requested.challenge as Uint8Array)], [4]);
  assert.deepEqual(requested.allowCredentials, [], 'none named');
  assert.equal(requested.userVerification, 'required');
});

test('what WebAuthn answers goes to the admin as base64url', () => {
  const registered = registrationJSON({
    id: 'AQI',
    rawId: bytes(1, 2).buffer,
    type: 'public-key',
    response: { attestationObject: bytes(3).buffer, clientDataJSON: bytes(4).buffer, getTransports: () => ['internal'] },
  } as unknown as PublicKeyCredential);
  assert.deepEqual(registered, {
    id: 'AQI',
    rawId: 'AQI',
    type: 'public-key',
    response: { attestationObject: 'Aw', clientDataJSON: 'BA', transports: ['internal'] },
  });
  const signed = assertionJSON({
    id: 'AQI',
    rawId: bytes(1, 2).buffer,
    type: 'public-key',
    response: { authenticatorData: bytes(5).buffer, clientDataJSON: bytes(6).buffer, signature: bytes(7).buffer, userHandle: null },
  } as unknown as PublicKeyCredential);
  assert.deepEqual(signed.response, { authenticatorData: 'BQ', clientDataJSON: 'Bg', signature: 'Bw', userHandle: null });
});

test('a refused passkey says whether this address offers none, or the person stopped it', () => {
  const refusal = (name: string, message = '') => passkeyRefusal(new DOMException(message, name));
  assert.equal(refusal('NotAllowedError', 'WebAuthn is not supported on sites with TLS certificate errors.'), 'untrusted');
  assert.equal(refusal('SecurityError', 'The operation is insecure.'), 'untrusted');
  assert.equal(refusal('NotAllowedError', 'The operation either timed out or was not allowed.'), 'cancelled');
  assert.equal(refusal('AbortError'), 'cancelled');
  assert.equal(refusal('InvalidStateError'), 'added');
  assert.equal(passkeyRefusal(new Error('forbidden')), 'other', 'the admin refused it, not the browser');
});

test('the recovery codes download holds every code, one per line', () => {
  const codes = ['aaaa-bbbb-cccc-dddd', 'eeee-ffff-gggg-hhhh'];
  const lines = recoveryFile(codes, 'easi.run recovery codes for https://localhost:9443', 'Each code works once.').split('\n');
  assert(lines[0]?.includes('https://localhost:9443'));
  for (const code of codes) assert(lines.includes(code), code);
});
