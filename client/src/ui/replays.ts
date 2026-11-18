// The replays page's order: the runs kept, told together by session, and the days a run is of.

import type { Run } from './api.ts';

/** One session's runs, newest first: a saved session's (every run of it), or one that was not saved (its only run). */
export interface Group {
  key: string;
  /** What its newest run was called. */
  name: string;
  saved: boolean;
  runs: Run[];
}

/** `runs` (newest first, as the admin lists them) by session, the session whose newest run is newest first. */
export function groupsOf(runs: readonly Run[]): Group[] {
  const groups = new Map<string, Group>();
  for (const run of runs) {
    const key = run.profile ? `saved:${run.profile}` : `run:${run.id}`;
    const group = groups.get(key);
    if (group) group.runs.push(run);
    else groups.set(key, { key, name: run.label, saved: Boolean(run.profile), runs: [run] });
  }
  return [...groups.values()];
}

/** The day `ms` is of where the page is read, as a date field writes one: `2026-11-18`. */
export function dayOf(ms: number): string {
  const at = new Date(ms);
  const two = (n: number) => String(n).padStart(2, '0');
  return `${at.getFullYear()}-${two(at.getMonth() + 1)}-${two(at.getDate())}`;
}

/** Whether `run` went on in the days `from` to `to` (each as `dayOf` writes one, or empty for no bound): any part of it. */
export function within(run: Run, from: string, to: string): boolean {
  return (!from || dayOf(run.stopped) >= from) && (!to || dayOf(run.started) <= to);
}
