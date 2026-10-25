import assert from 'node:assert/strict';
import { test } from 'node:test';
import { hashOf, PAGES, routeOf } from '../src/ui/route.ts';

test('each page opens at its own fragment, and that fragment names it again', () => {
  for (const page of PAGES) assert.deepEqual(routeOf(hashOf({ page })), { page });
  const session = { page: 'session', id: '0123456789ab' } as const;
  assert.equal(hashOf(session), '#session=0123456789ab');
  assert.deepEqual(routeOf(hashOf(session)), session);
});

test('a fragment that names no page opens the sessions', () => {
  for (const hash of ['', '#', '#session=', '#session=0123456789AB', '#Agents', '#agents/x', '#constructor', '#toString']) {
    assert.deepEqual(routeOf(hash), { page: 'sessions' }, hash);
  }
});
