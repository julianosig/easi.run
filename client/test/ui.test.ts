import assert from 'node:assert/strict';
import { test } from 'node:test';
import { trafficRate } from '../src/ui/ui.ts';

test('a traffic rate reads in kbit until it rounds to 1000, then in Mbit, never as 1000 kbit', () => {
  assert.deepEqual(trafficRate(0), { mega: false, rate: '0' });
  assert.deepEqual(trafficRate(412.4), { mega: false, rate: '412' });
  assert.deepEqual(trafficRate(999.4), { mega: false, rate: '999' });
  assert.deepEqual(trafficRate(999.5), { mega: true, rate: '1.0' });
  assert.deepEqual(trafficRate(999.6), { mega: true, rate: '1.0' });
  assert.deepEqual(trafficRate(1000), { mega: true, rate: '1.0' });
  assert.deepEqual(trafficRate(12_345), { mega: true, rate: '12.3' });
});
