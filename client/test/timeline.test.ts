import assert from 'node:assert/strict';
import { test } from 'node:test';
import { format } from '../../i18n/format.ts';
import type { TimelineEvent } from '../src/ui/api.ts';
import { leftAt, lines, markdown, merged, shown, summary } from '../src/ui/timeline.ts';
import english from '../messages/en/timeline.json';

const say = (key: string, params: Record<string, string | number> = {}) => format(english[key as keyof typeof english], params, 'en');
let seq = 0;
const at = (kind: TimelineEvent['kind'], fields: Record<string, unknown>): TimelineEvent => ({ id: ++seq, seq, at: 1_700_000_000_000 + seq * 1000, kind, ...fields });

test('each kind of event reads as a plain sentence, an action by what it did, never by what it typed', () => {
  const events = [
    at('session', { state: 'started', by: 'agent' }),
    at('page', { title: 'Sign in – Lumen Bank' }),
    at('call', { tool: 'run_steps', outcome: 'ok', ms: 1840, action: { steps: [{ tool: 'set_value', typed: 7 }, { tool: 'click', click: 'click', target: { role: 'button', name: 'Sign in' } }, { check: true }] } }),
    at('call', { tool: 'hotkey', outcome: 'ok', action: { keys: ['ctrl', 'l'] } }),
    at('call', { tool: 'click', outcome: 'error', error: 'no such element', action: { click: 'click' } }),
    at('call', { tool: 'click', outcome: 'blocked', times: 3, action: { click: 'click' } }),
    at('control', { to: 'person', reason: 'took' }),
    at('ask', { ask: 1, for: 'download', name: 'invoice.pdf', state: 'waiting' }),
    at('ask', { ask: 1, for: 'download', name: 'invoice.pdf', state: 'allowed' }),
    at('file', { file: 'saved', name: 'invoice.pdf', size: 3 }),
  ];
  const said = lines(events, say);
  assert.deepEqual(
    said.map((l) => l.text),
    [
      'The agent started the session',
      'On “Sign in – Lumen Bank”',
      'Ran 3 steps',
      'Pressed Ctrl+L',
      'Clicked',
      'Clicked',
      'You took control',
      'The agent asks to save invoice.pdf',
      'You allowed invoice.pdf',
      'The agent saved invoice.pdf',
    ],
  );
  assert.deepEqual(said[2]!.steps, ['Typed 7 characters', 'Clicked “Sign in”', 'Checked the page']);
  assert.equal(said[2]!.seconds, 1.8);
  assert.equal(said[4]!.outcome, 'Failed: no such element');
  assert.equal(said[5]!.outcome, 'Not run: a person had control');
  assert.equal(said[5]!.times, 3);
  assert.deepEqual(said.filter((l) => shown(l, 'problems')).map((l) => l.event.seq), [events[4]!.seq, events[5]!.seq, events[7]!.seq]);
  assert.deepEqual(said.filter((l) => shown(l, 'people')).map((l) => l.text), ['You took control', 'You allowed invoice.pdf']);
});

test('the same action right after itself is one line, counted; the summary counts what ran, went wrong and waits', () => {
  const looks = [1, 2, 3].map(() => at('call', { tool: 'get_window_state', outcome: 'ok', action: { looked: 'get_window_state' } }));
  const said = lines(looks, say);
  assert.equal(said.length, 1);
  assert.equal(said[0]!.times, 3);
  assert.equal(said[0]!.text, 'Looked at the screen');
  const events = [
    ...looks,
    at('page', { title: 'A' }),
    at('call', { tool: 'click', outcome: 'failed', error: 'timeout', action: { click: 'click' } }),
    at('ask', { ask: 4, for: 'transfer', name: 'a.png', state: 'waiting' }),
  ];
  assert.deepEqual(summary(events), { actions: 4, pages: 1, problems: 1, waiting: 1 });
});

test('when the person left is the last time nobody watched any more', () => {
  const events = [at('watch', { watching: true }), at('watch', { watching: false }), at('call', { tool: 'click', outcome: 'ok', action: {} }), at('watch', { watching: true })];
  assert.equal(leftAt(events), events[1]!.at);
  assert.equal(leftAt(events.slice(0, 1)), null);
});

test('the Markdown export has one item per line, with its steps and how it failed', () => {
  const events = [at('call', { tool: 'run_steps', outcome: 'error', error: 'expect failed', ms: 500, action: { steps: [{ tool: 'click', click: 'click' }] } })];
  const text = markdown('Timeline', lines(events, say), () => '12:00:00');
  assert.equal(text, '# Timeline\n\n- 12:00:00 · Ran 1 step — 0.5 s — **Failed: expect failed**\n  - Clicked\n');
});

test('a later page replaces an event it changed and adds the rest, in order', () => {
  const blocked = at('call', { tool: 'click', outcome: 'blocked', action: { click: 'click' } });
  const later = at('page', { title: 'B' });
  const retried = { ...blocked, seq: later.seq + 1, times: 2 };
  const known = merged([], [blocked], 10);
  assert.deepEqual(merged(known, [later, retried], 10).map((e) => [e.id, e.times ?? 1]), [[blocked.id, 2], [later.id, 1]]);
  assert.equal(merged(known, [later, retried], 1).length, 1);
});

test('a call whose caller left, or a failure kept without its text, still reads as a problem', () => {
  const said = lines([at('call', { tool: 'send_file', outcome: 'left', action: { files: 'send_file' } }), at('call', { tool: 'run_script', outcome: 'error', action: { script: 30 } })], say);
  assert.deepEqual(said.map((l) => l.outcome), ['Stopped: its caller left before it was done', 'Failed']);
  assert.equal(lines([at('page', { hidden: true })], say)[0]!.text, 'On a page whose title is not kept while you have control');
});
