// The admin page's translations: English in messages/en/<area>.json is the only text anyone edits; every other
// language follows it.
//
//   node scripts/i18n.mjs           translates what is new or changed in English, removes what English no longer has,
//                                   and writes the message types (ANTHROPIC_API_KEY; nothing to do: no network)
//   node scripts/i18n.mjs --check   fails if any language is missing, stale or extra, or the types are not current
//                                   (no network, no writes)
//   node scripts/i18n.mjs --types   writes the message types alone (no network), while English is being written
//   node scripts/i18n.mjs --redo=de,fr   translates every message of those languages again (all of them with --redo):
//                                   after a change to the instructions below, which no message's hash records
//
// Each translation keeps, beside it (messages/.sources/<locale>.json), the hash of the English it was made from: a
// key whose English changed, or that a language lacks, is translated again, alone; a key gone from English goes from
// every language, alone. The committed files are the only state: nothing is cached.
import { createHash } from 'node:crypto';
import { mkdir, readdir, readFile, rename, rm, writeFile } from 'node:fs/promises';

const root = new URL('..', import.meta.url).pathname;
const MESSAGES = `${root}messages`;
const SOURCES = `${MESSAGES}/.sources`;
const TYPES = `${root}src/i18n/keys.ts`;
const LOCALES = ['pt-BR', 'es', 'zh-CN', 'hi', 'fr', 'de', 'ja', 'ru', 'id', 'ko'];
const NAMES = {
  'pt-BR': 'Brazilian Portuguese',
  es: 'Spanish',
  'zh-CN': 'Simplified Chinese',
  hi: 'Hindi',
  fr: 'French',
  de: 'German',
  ja: 'Japanese',
  ru: 'Russian',
  id: 'Indonesian',
  ko: 'Korean',
};
const MODEL = 'claude-haiku-5-5';
/** How each language addresses the reader, the same everywhere: one register for the whole page. */
const REGISTER = {
  'pt-BR': 'address the reader as "você"',
  es: 'address the reader as "tú"',
  'zh-CN': 'address the reader as "你"',
  hi: 'address the reader as "आप"',
  fr: 'address the reader as "vous"',
  de: 'address the reader as "du" (lowercase), never "Sie"',
  ja: 'use the polite です/ます style; buttons and labels as short nouns or verbs',
  ru: 'address the reader as "вы" (lowercase)',
  id: 'address the reader as "Anda"',
  ko: 'use the polite 해요체 style; buttons and labels as short nouns or verbs',
};
/** Messages sent in one request: room for the longest answers within its output. */
const BATCH = 60;
const PLACEHOLDER = /\{([a-zA-Z][a-zA-Z0-9_]*)\}/g;
const CATEGORIES = ['zero', 'one', 'two', 'few', 'many', 'other'];

/** Every message of `locale` (`messages/<locale>/<area>.json`), by `area.name`. */
async function readCatalog(locale) {
  const dir = `${MESSAGES}/${locale}`;
  const catalog = {};
  let names = [];
  try {
    names = (await readdir(dir)).filter((n) => n.endsWith('.json')).sort();
  } catch {
    return catalog;
  }
  for (const name of names) {
    const area = name.slice(0, -'.json'.length);
    const entries = JSON.parse(await readFile(`${dir}/${name}`, 'utf8'));
    for (const [key, message] of Object.entries(entries)) catalog[`${area}.${key}`] = message;
  }
  return catalog;
}

async function readJson(path, otherwise) {
  try {
    return JSON.parse(await readFile(path, 'utf8'));
  } catch {
    return otherwise;
  }
}

/** Writes `path` whole, through a temporary file renamed over it. */
async function replace(path, text) {
  await writeFile(`${path}.tmp`, text);
  await rename(`${path}.tmp`, path);
}

/** A message's texts, a plural one's in CLDR order. */
const texts = (m) => (typeof m === 'string' ? [m] : CATEGORIES.filter((c) => c in m).map((c) => m[c]));
const names = (m) => [...new Set(texts(m).flatMap((t) => [...t.matchAll(PLACEHOLDER)].map((x) => x[1])))].sort();
/** The hash of an English message as it means, not as it is laid out: its kind, its forms in CLDR order. */
const hash = (m) =>
  createHash('sha256')
    .update(JSON.stringify(typeof m === 'string' ? m : CATEGORIES.filter((c) => c in m).map((c) => [c, m[c]])))
    .digest('hex')
    .slice(0, 16);

/** `message` without plural forms `locale` does not have (a model may add English's `one` to Japanese, say): harmless,
 * as `other` stays. Anything else as it is. */
function trimmed(message, locale) {
  if (typeof message !== 'object' || message === null || Array.isArray(message) || !('other' in message)) return message;
  const own = new Intl.PluralRules(locale).resolvedOptions().pluralCategories;
  return Object.fromEntries(Object.entries(message).filter(([c]) => own.includes(c)));
}

/** What is wrong with `message` as `locale`'s translation of `english`, if anything. */
function problem(english, message, locale) {
  if (typeof english === 'string') {
    if (typeof message !== 'string' || message.trim() === '') return 'not a text';
  } else {
    if (typeof message !== 'object' || message === null || Array.isArray(message)) return 'not a plural message';
    const wanted = new Intl.PluralRules(locale).resolvedOptions().pluralCategories.slice().sort();
    const given = Object.keys(message).sort();
    if (JSON.stringify(given) !== JSON.stringify(wanted)) return `plural forms ${given} instead of ${wanted}`;
    if (Object.values(message).some((t) => typeof t !== 'string' || t.trim() === '')) return 'an empty plural form';
  }
  // Every form of a plural message may leave out `count` (Japanese "one" may say it in words): the others must stay.
  const want = names(english).filter((n) => typeof english === 'string' || n !== 'count');
  const have = names(message).filter((n) => typeof english === 'string' || n !== 'count');
  return JSON.stringify(want) === JSON.stringify(have) ? null : `placeholders ${have} instead of ${want}`;
}

/** The languages `--redo` names (every one when it names none), translated again whole. */
const redoArg = process.argv.find((a) => a === '--redo' || a.startsWith('--redo='));
const redo = redoArg === undefined ? [] : redoArg === '--redo' ? LOCALES : redoArg.slice('--redo='.length).split(',');
for (const locale of redo) if (!LOCALES.includes(locale)) throw new Error(`--redo: no language ${locale}`);

/** Per locale: what to translate (new or changed in English) and what to remove (gone from English). */
async function plan(english) {
  const plans = [];
  for (const locale of LOCALES) {
    const catalog = await readCatalog(locale);
    const sources = await readJson(`${SOURCES}/${locale}.json`, {});
    const pending = Object.keys(english).filter((k) => redo.includes(locale) || !(k in catalog) || sources[k] !== hash(english[k]));
    const removed = Object.keys(catalog).filter((k) => !(k in english));
    const broken = Object.keys(catalog)
      .filter((k) => k in english && !pending.includes(k))
      .map((k) => [k, problem(english[k], catalog[k], locale)])
      .filter(([, p]) => p);
    plans.push({ locale, catalog, sources, pending, removed, broken });
  }
  return plans;
}

/** The types of every message's parameters, from English: what `t()` checks at compile time. */
function types(english) {
  const lines = Object.keys(english)
    .sort()
    .map((key) => {
      const m = english[key];
      const params = names(m).filter((n) => n !== 'count' || typeof m === 'string');
      const fields = [...(typeof m === 'string' ? [] : ['count: number']), ...params.map((n) => `${n}: string | number`)];
      return `  ${JSON.stringify(key)}: ${fields.length === 0 ? 'Record<string, never>' : `{ ${fields.join('; ')} }`};`;
    });
  return `// Written by scripts/i18n.mjs from messages/en: never edited by hand.\n\nexport interface MessageParams {\n${lines.join('\n')}\n}\n`;
}

const SYSTEM = `You translate the user interface of easi.run, a self-hosted product that gives agents a real Chrome
browser a person can watch live and take over. The reader is the person who administers it: technical, but write
plainly. Keep these as they are: easi.run, Chrome, MCP, API, SDK, token (may be translated if the language has a common
equivalent in software), proxy, WebM, HTTPS, URL, Developer tools names, product and file names, and anything inside
backticks. Never translate {placeholders}: keep each one exactly, once, where the sentence needs it. Match the English's
length and tone: short labels stay short (a button is one or two words), sentences stay one sentence. Use the
language's usual typography (quotes, ellipsis, non-breaking spaces where usual).
A message is either a text or a plural message: an object of CLDR plural categories. For a plural message, answer with
exactly the categories you are given for the language, each a full text; {count} is the number.
Answer with one JSON object only, no prose: each key you were given, its translation.`;

/** Batches translated at once per language. */
const CONCURRENCY = 6;

/** `pending` English messages translated into `locale` by Claude, checked, in batches run side by side; `done` takes
 * each batch as it ends. Throws, once every batch ended, if any answer stayed unusable. */
async function translate(locale, english, pending, key, done) {
  const categories = new Intl.PluralRules(locale).resolvedOptions().pluralCategories;
  const batches = [];
  for (let at = 0; at < pending.length; at += BATCH) {
    batches.push(Object.fromEntries(pending.slice(at, at + BATCH).map((k) => [k, english[k]])));
  }
  const one = async (batch) => {
    let feedback = '';
    for (let attempt = 0; ; attempt++) {
      const ask = `Language: ${NAMES[locale]} (${locale}); ${REGISTER[locale]}. Plural categories for this language: ${categories.join(', ')}.\n${feedback}Messages:\n${JSON.stringify(batch, null, 1)}`;
      const response = await fetch('https://api.anthropic.com/v1/messages', {
        method: 'POST',
        headers: { 'x-api-key': key, 'anthropic-version': '2023-06-01', 'content-type': 'application/json' },
        body: JSON.stringify({ model: MODEL, max_tokens: 16000, system: SYSTEM, messages: [{ role: 'user', content: ask }] }),
      });
      if (!response.ok) throw new Error(`${locale}: the translator answered ${response.status}: ${(await response.text()).slice(0, 300)}`);
      const answer = (await response.json()).content.map((c) => c.text ?? '').join('');
      let parsed;
      try {
        parsed = JSON.parse(answer.slice(answer.indexOf('{'), answer.lastIndexOf('}') + 1));
      } catch {
        parsed = null;
      }
      const errors = [];
      if (parsed === null || typeof parsed !== 'object') errors.push('the answer is not one JSON object');
      else {
        for (const k of Object.keys(parsed)) parsed[k] = trimmed(parsed[k], locale);
        for (const k of Object.keys(batch)) {
          if (!(k in parsed)) errors.push(`${k}: missing`);
          else {
            const p = problem(batch[k], parsed[k], locale);
            if (p) errors.push(`${k}: ${p}`);
          }
        }
        for (const k of Object.keys(parsed)) if (!(k in batch)) errors.push(`${k}: not asked for`);
      }
      if (errors.length === 0) return done(Object.fromEntries(Object.keys(batch).map((k) => [k, parsed[k]])));
      if (attempt === 2) throw new Error(`${locale}: unusable translations:\n  ${errors.join('\n  ')}`);
      feedback = `Your previous answer had these problems; fix them:\n${errors.join('\n')}\n`;
    }
  };
  const queue = [...batches];
  const failures = [];
  await Promise.all(
    Array.from({ length: Math.min(CONCURRENCY, queue.length) }, async () => {
      while (queue.length) {
        try {
          await one(queue.shift());
        } catch (e) {
          failures.push(e.message);
        }
      }
    }),
  );
  if (failures.length) throw new Error(failures.join('\n'));
}

/** Writes `catalog` as `locale`'s area files (keys sorted, one file per area English has) and its sources. */
async function write(locale, catalog, sources) {
  const dir = `${MESSAGES}/${locale}`;
  await mkdir(dir, { recursive: true });
  const areas = {};
  for (const [key, message] of Object.entries(catalog)) {
    const dot = key.indexOf('.');
    (areas[key.slice(0, dot)] ??= {})[key.slice(dot + 1)] = message;
  }
  for (const name of await readdir(dir)) {
    if (name.endsWith('.json') && !(name.slice(0, -5) in areas)) await rm(`${dir}/${name}`);
  }
  for (const [area, entries] of Object.entries(areas)) {
    const sorted = Object.fromEntries(Object.keys(entries).sort().map((k) => [k, entries[k]]));
    await replace(`${dir}/${area}.json`, `${JSON.stringify(sorted, null, 2)}\n`);
  }
  await mkdir(SOURCES, { recursive: true });
  const sorted = Object.fromEntries(Object.keys(sources).sort().map((k) => [k, sources[k]]));
  await replace(`${SOURCES}/${locale}.json`, `${JSON.stringify(sorted, null, 2)}\n`);
}

const english = await readCatalog('en');
for (const [key, message] of Object.entries(english)) {
  if (typeof message !== 'string' && (typeof message !== 'object' || !('other' in message))) {
    throw new Error(`en: ${key} is neither a text nor a plural message with "other"`);
  }
}
const plans = await plan(english);
const currentTypes = await readFile(TYPES, 'utf8').catch(() => '');

if (process.argv.includes('--types')) {
  if (currentTypes !== types(english)) await replace(TYPES, types(english));
} else if (process.argv.includes('--check')) {
  const failures = [];
  if (currentTypes !== types(english)) failures.push('src/i18n/keys.ts is not current: run node scripts/i18n.mjs');
  for (const p of plans) {
    if (p.pending.length) failures.push(`${p.locale}: ${p.pending.length} missing or stale (${p.pending.slice(0, 5).join(', ')})`);
    if (p.removed.length) failures.push(`${p.locale}: ${p.removed.length} no longer in English (${p.removed.slice(0, 5).join(', ')})`);
    for (const [k, why] of p.broken) failures.push(`${p.locale}: ${k}: ${why}`);
  }
  if (failures.length) {
    console.error(`translations are not current:\n  ${failures.join('\n  ')}`);
    process.exit(1);
  }
  console.log(`${Object.keys(english).length} messages, current in ${LOCALES.length} languages`);
} else {
  if (currentTypes !== types(english)) await replace(TYPES, types(english));
  const work = plans.filter((p) => p.pending.length || p.removed.length || p.broken.length);
  const translating = work.some((p) => p.pending.length || p.broken.length);
  const key = process.env.ANTHROPIC_API_KEY;
  if (translating && !key) throw new Error('ANTHROPIC_API_KEY is needed to translate');
  const started = Date.now();
  const results = await Promise.allSettled(
    work.map(async (p) => {
      const redo = [...new Set([...p.pending, ...p.broken.map(([k]) => k)])];
      const catalog = { ...p.catalog };
      const sources = { ...p.sources };
      for (const k of p.removed) {
        delete catalog[k];
        delete sources[k];
      }
      for (const k of Object.keys(sources)) if (!(k in english)) delete sources[k];
      let made = 0;
      // Each batch is written as it ends, so a later failure keeps what was done.
      let writing = write(p.locale, catalog, sources);
      const done = (batch) => {
        Object.assign(catalog, batch);
        for (const k of Object.keys(batch)) sources[k] = hash(english[k]);
        made += Object.keys(batch).length;
        writing = writing.then(() => write(p.locale, catalog, sources));
        return writing;
      };
      try {
        if (redo.length) await translate(p.locale, english, redo, key, done);
      } finally {
        await writing;
        console.log(`${p.locale}: ${made} translated, ${p.removed.length} removed`);
      }
    }),
  );
  if (work.length === 0) console.log('nothing to do');
  else console.log(`done in ${((Date.now() - started) / 1000).toFixed(1)} s`);
  const failed = results.filter((r) => r.status === 'rejected');
  if (failed.length) {
    for (const f of failed) console.error(f.reason.message);
    process.exit(1);
  }
}
