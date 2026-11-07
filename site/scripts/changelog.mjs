// Checks public/changelog.json, which https://easi.run/changelog.json serves and an installation's update notice reads:
// what each release changed for the people using it, newest first, in every language the pages speak.
//   {"releases": [{"commit": <the 40-digit commit install.sh pins>, "date": "YYYY-MM-DD", "changes": [{<locale>: text}]}]}
import { readFile } from 'node:fs/promises';
import { LOCALES } from '../../i18n/locales.ts';

const path = new URL('../public/changelog.json', import.meta.url);
const { releases } = JSON.parse(await readFile(path, 'utf8'));
const wrong = (why) => {
  console.error(`public/changelog.json: ${why}`);
  process.exit(1);
};
if (!Array.isArray(releases)) wrong('no releases list');
const seen = new Set();
let after = '9999-12-31';
for (const [i, release] of releases.entries()) {
  const { commit, date, changes, ...rest } = release ?? {};
  if (Object.keys(rest).length) wrong(`release ${i}: unknown ${Object.keys(rest).join(', ')}`);
  if (typeof commit !== 'string' || !/^[0-9a-f]{40}$/.test(commit) || seen.has(commit)) wrong(`release ${i}: commit`);
  seen.add(commit);
  if (typeof date !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(date) || date > after) wrong(`release ${i}: date`);
  after = date;
  if (!Array.isArray(changes) || changes.length === 0) wrong(`release ${i}: no changes`);
  for (const [j, change] of changes.entries()) {
    const said = Object.keys(change ?? {});
    if (said.length !== LOCALES.length || LOCALES.some((l) => typeof change[l] !== 'string' || !change[l].trim())) {
      wrong(`release ${i}, change ${j}: needs exactly ${LOCALES.join(', ')}`);
    }
  }
}
