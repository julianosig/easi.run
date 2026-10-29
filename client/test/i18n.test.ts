import assert from 'node:assert/strict';
import { test } from 'node:test';
import { format } from '../../i18n/format.ts';
import { match } from '../../i18n/locales.ts';

test('placeholders are filled, numbers in the locale’s own digits and grouping', () => {
  assert.equal(format('Saved {when} · {size}', { when: 'today', size: '2 MB' }, 'en'), 'Saved today · 2 MB');
  assert.equal(format('{count} items', { count: 12345 }, 'de'), '12.345 items');
  assert.equal(format('Hello {name}', {}, 'en'), 'Hello {name}', 'a missing parameter stays as written');
});

test('a plural message takes the form of its count in its locale', () => {
  const ru = { one: '{count} сессия', few: '{count} сессии', many: '{count} сессий', other: '{count} сессии' };
  assert.deepEqual([1, 3, 5, 21].map((count) => format(ru, { count }, 'ru')), ['1 сессия', '3 сессии', '5 сессий', '21 сессия']);
  assert.equal(format({ other: '{count} 個' }, { count: 1 }, 'ja'), '1 個');
});

test('the browser’s languages pick the first the page speaks, else English', () => {
  assert.equal(match(['fr-CA', 'en']), 'fr');
  assert.equal(match(['pt-PT']), 'pt-BR');
  assert.equal(match(['zh-TW', 'en']), 'zh-CN');
  assert.equal(match(['nl', 'sv']), 'en');
  assert.equal(match([]), 'en');
});
