import assert from 'node:assert/strict';
import { test } from 'node:test';
import { demoOf } from '../src/ui/demos.ts';

test('a demo name reads only the known pages, never an inherited property', () => {
  assert.equal(demoOf('typing').title, 'Typing');
  for (const name of ['constructor', 'toString', 'hasOwnProperty', 'shop']) {
    assert.deepEqual(demoOf(name), { title: name, line: '', icon: 'M5 12h14' }, name);
  }
});
