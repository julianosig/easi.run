import assert from 'node:assert/strict';
import { test } from 'node:test';
import { format } from '../src/i18n/format.ts';
import { countryName, type Exit, exitText, flagOf, languageName, NAME_PATTERN, reasonKey, zoneName } from '../src/ui/proxies.ts';
import english from '../messages/en/proxies.json';

/** A check failure's reason as the page reads it in English. */
const reasonText = (reason: unknown) => format(english[reasonKey(reason).slice('proxies.'.length) as keyof typeof english], {}, 'en');

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
  assert.equal(languageName('pt-BR', 'en', [{ tag: 'pt-BR', name: 'Portuguese (Brazil)' }]), 'Brazilian Portuguese', 'the browser’s name first');
  assert.equal(languageName('zz', 'en', [{ tag: 'zz', name: 'Listed' }]), 'Listed', 'the admin’s name for a tag the browser does not know');
  assert.equal(languageName('de', 'en'), 'German');
  assert.equal(languageName('de', 'fr'), 'allemand', 'in the page’s language');
  assert.equal(languageName('not a tag!', 'en'), 'not a tag!');
});

test('a country reads in the page’s language by its code, else as the admin named it', () => {
  assert.equal(countryName('DE', 'Germany', 'fr'), 'Allemagne');
  assert.equal(countryName('de', 'Germany', 'en'), 'Germany');
  assert.equal(countryName(null, 'Germany', 'fr'), 'Germany', 'no code: the admin’s name');
  assert.equal(countryName('not a code', null, 'fr'), null);
  const exit: Exit = { ip: '203.0.113.7', country: 'DE', country_name: 'Germany', region: 'Hesse', city: 'Frankfurt' };
  assert.equal(exitText(exit, 'fr'), '🇩🇪 Frankfurt, Hesse, Allemagne · 203.0.113.7');
});

test('a proxy name passes the field only as the admin takes it: 1 to 32 of a-z, 0-9 and -', () => {
  // As a browser compiles an input's pattern: whole, with the v flag.
  const name = new RegExp(`^(?:${NAME_PATTERN})$`, 'v');
  for (const ok of ['frankfurt', 'br-home-2', 'a', 'x'.repeat(32)]) assert.equal(name.test(ok), true, ok);
  for (const bad of ['', 'Frankfurt', 'são-paulo', 'a b', 'a_b', 'x'.repeat(33)]) assert.equal(name.test(bad), false, bad);
});
