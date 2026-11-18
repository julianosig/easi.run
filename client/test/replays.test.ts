import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { Run } from '../src/ui/api.ts';
import { dayOf, groupsOf, within } from '../src/ui/replays.ts';

/** A run that started at `started` (a local date and time) and went on for `minutes`. */
const run = (id: string, profile: string | null, started: string, minutes: number, label = profile ?? id): Run => {
  const at = new Date(started).getTime();
  return { id, label, profile, started: at, stopped: at + minutes * 60_000 };
};

test('runs are told together by saved session, each session where its newest run is', () => {
  const runs = [
    run('a3', 'work', '2026-11-18T10:00', 5, 'Work (renamed)'),
    run('b1', null, '2026-11-18T09:00', 5, 'https://shop.example/'),
    run('a2', 'work', '2026-11-17T10:00', 5, 'Work'),
    run('c1', null, '2026-11-16T09:00', 5, 'https://shop.example/'),
    run('a1', 'work', '2026-11-15T10:00', 5, 'Work'),
  ];
  const groups = groupsOf(runs);
  assert.deepEqual(
    groups.map((g) => [g.name, g.saved, g.runs.map((r) => r.id)]),
    [
      ['Work (renamed)', true, ['a3', 'a2', 'a1']],
      // Two sessions that were not saved are two sessions, whatever page each started at.
      ['https://shop.example/', false, ['b1']],
      ['https://shop.example/', false, ['c1']],
    ],
  );
  assert.equal(new Set(groups.map((g) => g.key)).size, 3);
  assert.deepEqual(groupsOf([]), []);
});

test('a day is the day where the page is read, as a date field writes it', () => {
  assert.equal(dayOf(new Date(2026, 0, 5, 0, 0).getTime()), '2026-01-05');
  assert.equal(dayOf(new Date(2026, 10, 18, 23, 59, 59).getTime()), '2026-11-18');
});

test('the dates keep a run that went on in any part of them', () => {
  const overnight = run('x', null, '2026-11-17T23:30', 60);
  for (const [from, to, kept] of [
    ['', '', true],
    ['2026-11-17', '2026-11-17', true],
    ['2026-11-18', '2026-11-18', true],
    ['2026-11-18', '', true],
    ['', '2026-11-17', true],
    ['2026-11-19', '', false],
    ['', '2026-11-16', false],
    ['2026-11-10', '2026-11-16', false],
  ] as const) {
    assert.equal(within(overnight, from, to), kept, `${from}..${to}`);
  }
});
