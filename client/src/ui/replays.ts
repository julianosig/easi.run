// The replays page's order: the runs kept, told together by session, and the days a run is of.

import type { Run } from './api.ts';

/** One session's runs, newest first: a saved session's (every run of it), or the runs called the same of one whose
 * archives do not say which saved session they were of. */
export interface Group {
  key: string;
  /** What its newest run was called. */
  name: string;
  saved: boolean;
  runs: Run[];
}

/** `runs` (newest first, as the admin lists them) by session, the session whose newest run is newest first. A run
 * that says its saved session goes with that session's other runs, whatever each was called (a session renamed stays
 * one). A run that says none (an archive kept before archives said it, or a session that was not saved) goes with
 * the runs called the same: with a saved session's, when one was ever called that, else among themselves. */
export function groupsOf(runs: readonly Run[]): Group[] {
  const groups = new Map<string, Group>();
  // What each saved session's runs were called, to its group's key.
  const called = new Map<string, string>();
  for (const run of runs) {
    if (run.profile && !called.has(run.label)) called.set(run.label, `saved:${run.profile}`);
  }
  for (const run of runs) {
    const key = run.profile ? `saved:${run.profile}` : (called.get(run.label) ?? `called:${run.label}`);
    const group = groups.get(key);
    if (group) group.runs.push(run);
    else groups.set(key, { key, name: run.label, saved: key.startsWith('saved:'), runs: [run] });
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
