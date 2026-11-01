// What a session's timeline says (admin/src/timeline.rs), in the page's words: one line per event, who it was about,
// how it went, a summary of what happened since the person last looked, and the whole as Markdown an agent can follow.
// Pure: the words come from `say`, the page's catalog (or English in the tests).

import type { TimelineEvent } from './api.ts';

/** A message of the `timeline.*` catalog, by its key without the area, with its parameters. */
export type Say = (key: string, params?: Record<string, string | number>) => string;

/** Who an event is about: the agent's own actions, people (control, watching, answers), or the page and the session. */
export type Actor = 'agent' | 'people' | 'page';
/** How it went: a failure or refusal is a problem; an ask waiting for an answer, waiting. */
export type Tone = 'plain' | 'problem' | 'waiting' | 'done';

/** One line of the timeline: consecutive identical successful actions are one line, counted. */
export interface Line {
  event: TimelineEvent;
  /** The same action repeated, or a refused call retried. */
  times: number;
  actor: Actor;
  tone: Tone;
  text: string;
  /** How it ended, when it did not go through: what the tool said, or why it was not run. */
  outcome?: string | undefined;
  /** A batch's steps, each said. */
  steps?: string[] | undefined;
  /** How long it took, in seconds, when it was an action that ran. */
  seconds?: number | undefined;
}

const str = (v: unknown): string => (typeof v === 'string' ? v : '');
const num = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0);

/** What an action did, from the summary the admin keeps of it (`timeline::summary`). */
export function action(tool: string, a: Record<string, unknown>, say: Say): string {
  if (typeof a.click === 'string') {
    const verb = say(`click.${['click', 'double_click', 'right_click'].includes(a.click) ? a.click : 'click'}`);
    const name = str((a.target as Record<string, unknown> | undefined)?.name);
    return name ? say('click.target', { action: verb, name }) : verb;
  }
  if ('typed' in a) return say('typed', { count: num(a.typed) });
  if (Array.isArray(a.keys)) {
    const keys = a.keys.map((k) => (k === 'character' ? say('key.character') : keyName(str(k)))).join('+');
    return say('keys', { keys });
  }
  if ('scroll' in a) {
    const direction = str(a.scroll);
    return ['up', 'down', 'left', 'right'].includes(direction) ? say('scroll', { direction: say(`direction.${direction}`) }) : say('scroll.plain');
  }
  if (a.pointer === 'drag' || a.pointer === 'move_cursor') return say(`pointer.${a.pointer}`);
  if (typeof a.looked === 'string') return say(a.looked === 'list_windows' || a.looked === 'list_files' ? `looked.${a.looked}` : 'looked.screen');
  if (Array.isArray(a.steps)) return say('steps', { count: a.steps.length });
  if ('script' in a) return say('script');
  if (a.files === 'save_download' || a.files === 'send_file' || a.files === 'choose_files') return say(`files.${a.files}`);
  return say('used', { tool });
}

/** A named key as people write it: `ctrl` as Ctrl, `enter` as Enter. */
function keyName(key: string): string {
  return key.length <= 1 ? key.toUpperCase() : key[0]!.toUpperCase() + key.slice(1);
}

/** One event as a line, alone. */
export function line(event: TimelineEvent, say: Say): Line {
  const e = event as TimelineEvent & Record<string, unknown>;
  const base = { event, times: num(e.times) || 1 };
  switch (e.kind) {
    case 'call': {
      const a = (e.action ?? {}) as Record<string, unknown>;
      const outcome = str(e.outcome);
      const text = action(str(e.tool), a, say);
      const steps = Array.isArray(a.steps)
        ? a.steps.map((s) => (s && typeof s === 'object' && (s as Record<string, unknown>).check ? say('step.check') : action(str((s as Record<string, unknown>).tool), s as Record<string, unknown>, say)))
        : undefined;
      const seconds = typeof e.ms === 'number' ? Math.round(e.ms / 100) / 10 : undefined;
      const said =
        outcome === 'error' || outcome === 'failed'
          ? e.error
            ? say(`outcome.${outcome}`, { error: str(e.error) })
            : say(`outcome.${outcome}.plain`)
          : outcome === 'blocked' || outcome === 'busy' || outcome === 'withdrawn' || outcome === 'left'
            ? say(`outcome.${outcome}`)
            : undefined;
      return { ...base, actor: 'agent', tone: said ? 'problem' : 'plain', text, outcome: said, steps, seconds };
    }
    case 'control':
      return { ...base, actor: 'people', tone: 'plain', text: say(`control.${['opened', 'took', 'gave', 'left'].includes(str(e.reason)) ? e.reason : 'took'}`) };
    case 'watch':
      return { ...base, actor: 'people', tone: 'plain', text: say(e.watching ? 'watch.on' : 'watch.off') };
    case 'ask': {
      const state = str(e.state);
      const name = str(e.name);
      if (state === 'waiting') return { ...base, actor: 'agent', tone: 'waiting', text: say(e.for === 'transfer' ? 'ask.waiting.transfer' : 'ask.waiting.download', { name }) };
      const known = ['allowed', 'refused', 'expired', 'cancelled'].includes(state) ? state : 'cancelled';
      return { ...base, actor: known === 'allowed' || known === 'refused' ? 'people' : 'agent', tone: known === 'allowed' ? 'done' : known === 'refused' ? 'plain' : 'problem', text: say(`ask.${known}`, { name }) };
    }
    case 'file': {
      const kind = str(e.file);
      const name = str(e.name);
      if (kind === 'chosen') return { ...base, actor: 'agent', tone: 'done', text: say('file.chosen', { count: num(e.count) }) };
      if (kind === 'saved' || kind === 'stored') return { ...base, actor: 'agent', tone: 'done', text: say(`file.${kind}`, { name }) };
      return { ...base, actor: 'page', tone: 'plain', text: kind === 'chooser' ? say('file.chooser') : say('file.offered', { name }) };
    }
    case 'page': {
      const title = str(e.title);
      const text = e.hidden ? say('page.hidden') : title ? say('page', { title }) : say('page.untitled');
      return { ...base, actor: 'page', tone: 'plain', text };
    }
    default:
      return { ...base, actor: 'page', tone: 'plain', text: e.state === 'stopped' ? say('session.stopped') : say(e.by === 'agent' ? 'session.started.agent' : 'session.started.person') };
  }
}

/** `known` with `events` (a later page of the same timeline) taken in: an event it has already is replaced by its
 * change (a refusal retried again), the rest added; in order of their numbers, the latest `max`. */
export function merged(known: readonly TimelineEvent[], events: readonly TimelineEvent[], max: number): TimelineEvent[] {
  const byId = new Map(known.map((e) => [e.id, e]));
  for (const e of events) byId.set(e.id, e);
  return [...byId.values()].sort((a, b) => a.id - b.id).slice(-max);
}

/** The events as lines, in order: an action that went through repeated right after itself (the agent looking at the
 * screen again, say) is one line, counted. */
export function lines(events: readonly TimelineEvent[], say: Say): Line[] {
  const out: Line[] = [];
  for (const event of events) {
    const next = line(event, say);
    const last = out.at(-1);
    if (last && next.event.kind === 'call' && last.event.kind === 'call' && next.tone === 'plain' && last.tone === 'plain' && next.text === last.text && !next.steps) {
      last.times += next.times;
      continue;
    }
    out.push(next);
  }
  return out;
}

/** Which lines a filter shows: everything, the agent's actions, what people did, or what went wrong or waits. */
export type Filter = 'all' | 'agent' | 'people' | 'problems';
export function shown(line: Line, filter: Filter): boolean {
  if (filter === 'all') return true;
  if (filter === 'problems') return line.tone === 'problem' || line.tone === 'waiting';
  return line.actor === filter;
}

/** What happened in `events` (those since the person last looked): actions, pages, problems, and questions still
 * waiting for an answer. */
export interface Summary {
  actions: number;
  pages: number;
  problems: number;
  waiting: number;
}
export function summary(events: readonly TimelineEvent[]): Summary {
  const s: Summary = { actions: 0, pages: 0, problems: 0, waiting: 0 };
  const open = new Set<number>();
  for (const e of events as readonly (TimelineEvent & Record<string, unknown>)[]) {
    if (e.kind === 'call') {
      const outcome = str(e.outcome);
      if (outcome === 'ok' || outcome === 'error' || outcome === 'failed' || outcome === 'left') s.actions += 1;
      if (outcome !== 'ok') s.problems += 1;
    } else if (e.kind === 'page') s.pages += 1;
    else if (e.kind === 'ask') {
      if (e.state === 'waiting') open.add(num(e.ask));
      else {
        open.delete(num(e.ask));
        if (e.state === 'expired') s.problems += 1;
      }
    }
  }
  s.waiting = open.size;
  return s;
}

/** When the person last left the session (nobody watched it any more), if the timeline says so. */
export function leftAt(events: readonly TimelineEvent[]): number | null {
  let at: number | null = null;
  for (const e of events) if (e.kind === 'watch' && !e.watching) at = e.at;
  return at;
}

/** The lines as Markdown: a heading, then one item per line with its time, how it ended and its steps. */
export function markdown(title: string, lines: readonly Line[], time: (ms: number) => string): string {
  const out = [`# ${title}`, ''];
  for (const l of lines) {
    const times = l.times > 1 ? ` (×${l.times})` : '';
    const took = l.seconds !== undefined ? ` — ${l.seconds} s` : '';
    out.push(`- ${time(l.event.at)} · ${l.text}${times}${took}${l.outcome ? ` — **${l.outcome}**` : ''}`);
    for (const step of l.steps ?? []) out.push(`  - ${step}`);
  }
  return out.join('\n') + '\n';
}
