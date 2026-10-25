import assert from 'node:assert/strict';
import { test } from 'node:test';
import { flagOf, languageName, NAME_PATTERN, reasonText, zoneName } from '../src/ui/proxies.ts';

test('each check failure reads in plain words, an unknown one as unreachable', () => {
  assert.equal(reasonText('auth'), 'Wrong user name or password');
  assert.equal(reasonText('protocol'), 'Not a proxy at that address');
  assert.equal(reasonText('unexpected'), 'The proxy answered, but not as expected');
  assert.equal(reasonText('something-new'), reasonText('unreachable'));
  assert.equal(reasonText(undefined), reasonText('unreachable'));
  assert.equal(reasonText('toString'), reasonText('unreachable'));
});

test('a country code becomes its flag, anything else nothing', () => {
  assert.equal(flagOf('BR'), '🇧🇷');
  assert.equal(flagOf('de'), '🇩🇪');
  for (const code of ['', 'B', 'BRA', '1A', null, undefined]) assert.equal(flagOf(code), '');
});

test('a time zone reads as its city, a language by its listed name or the browser’s', () => {
  assert.equal(zoneName('America/Sao_Paulo'), 'Sao Paulo');
  assert.equal(zoneName('America/Argentina/Buenos_Aires'), 'Buenos Aires');
  assert.equal(zoneName('UTC'), 'UTC');
  assert.equal(languageName('pt-BR', [{ tag: 'pt-BR', name: 'Portuguese (Brazil)' }]), 'Portuguese (Brazil)');
  assert.equal(languageName('de'), 'German');
  assert.equal(languageName('not a tag!'), 'not a tag!');
});

test('a proxy name passes the field only as the admin takes it: 1 to 32 of a-z, 0-9 and -', () => {
  // As a browser compiles an input's pattern: whole, with the v flag.
  const name = new RegExp(`^(?:${NAME_PATTERN})$`, 'v');
  for (const ok of ['frankfurt', 'br-home-2', 'a', 'x'.repeat(32)]) assert.equal(name.test(ok), true, ok);
  for (const bad of ['', 'Frankfurt', 'são-paulo', 'a b', 'a_b', 'x'.repeat(33)]) assert.equal(name.test(bad), false, bad);
});
