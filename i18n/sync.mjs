// A package's translations (the admin page's, client/; the site's, site/): English in <package>/messages/en/<area>.json
// is the only text anyone edits; every other language follows it. From the package's directory:
//
//   node ../i18n/sync.mjs           translates what is new or changed in English, removes what English no longer has,
//                                   and writes the message types (ANTHROPIC_API_KEY; nothing to do: no network)
//   node ../i18n/sync.mjs --check   fails if any language is missing, stale or extra, or the types are not current
//                                   (no network, no writes)
//   node ../i18n/sync.mjs --types   writes the message types alone (no network), while English is being written
//   node ../i18n/sync.mjs --redo=de,fr   translates every message of those languages again (all of them with
//                                   --redo): after a change to the instructions below or to the package's brief, which no
//                                   message's hash records
//
// Each translation keeps, beside it (messages/.sources/<locale>.json), the hash of the English it was made from: a
// key whose English changed, or that a language lacks, is translated again, alone; a key gone from English goes from
// every language, alone. The committed files are the only state: nothing is cached. The package's
// messages/brief.txt says what it is and who reads it, for the translator.
import { createHash } from 'node:crypto';
import { mkdir, readdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { LOCALES as ALL, SOURCE } from './locales.ts';
import { isRich, parseRich, richShape } from './rich.ts';

/** `--name=value`'s value, else `otherwise`. */
const option = (name, otherwise) => process.argv.find((a) => a.startsWith(`--${name}=`))?.slice(name.length + 3) ?? otherwise;
const root = `${process.cwd()}/`;
// A package's messages and types, or (--messages, --types-out) another catalog of it: the site's docs keep theirs apart.
const MESSAGES = `${root}${option('messages', 'messages')}`;
const SOURCES = `${MESSAGES}/.sources`;
const TYPES = `${root}${option('types-out', 'src/i18n/keys.ts')}`;
const LOCALES = ALL.filter((l) => l !== SOURCE);
/** Each language's English name, for the translator. */
const NAMES = Object.fromEntries(LOCALES.map((l) => [l, new Intl.DisplayNames(['en'], { type: 'language' }).of(l)]));
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
/** A text without its <code> elements, whose braces are literal (see rich.ts). */
const outsideCode = (t) => t.replace(/<code>[\s\S]*?<\/code>/g, '');
const names = (m) => [...new Set(texts(m).flatMap((t) => [...outsideCode(t).matchAll(PLACEHOLDER)].map((x) => x[1])))].sort();
/** Whether `text` is in rich.ts's grammar. */
const parses = (text) => {
  try {
    parseRich(text);
    return true;
  } catch {
    return false;
  }
};
/** Each form's placeholders with how many times it has each: a translation keeps them exactly. */
const counts = (t) => [...outsideCode(t).matchAll(PLACEHOLDER)].map((x) => x[1]).sort().join(',');
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
  if (JSON.stringify(want) !== JSON.stringify(have)) return `placeholders ${have} instead of ${want}`;
  // A text keeps each placeholder as many times as English has it; each form of a plural message, as many as English's
  // `other` has (`count` aside).
  if (typeof english === 'string') {
    if (counts(message) !== counts(english)) return `placeholders ${counts(message)} instead of ${counts(english)}`;
    // Markup: a text that parses as markup (see rich.ts) is one a page may draw as such, so its translation must parse
    // too, and keep the same code, links (each with its code), emphasis and placeholders, properly nested.
    if (parses(english)) {
      let shape;
      try {
        shape = richShape(parseRich(message));
      } catch (e) {
        return `markup: ${e.message}`;
      }
      if (shape !== richShape(parseRich(english))) return `markup ${shape} instead of ${richShape(parseRich(english))}`;
    }
  } else {
    const want = counts(english.other.replace(/\{count\}/g, ''));
    for (const [form, text] of Object.entries(message)) {
      if (counts(text.replace(/\{count\}/g, '')) !== want) return `${form}: placeholders ${counts(text)} instead of ${want} besides {count}`;
    }
  }
  return null;
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
  return `// Written by i18n/sync.mjs from messages/en: never edited by hand.\n\nexport interface MessageParams {\n${lines.join('\n')}\n}\n`;
}

const BRIEF = (await readFile(`${MESSAGES}/brief.txt`, 'utf8')).trim();
const SYSTEM = `${BRIEF}
Keep these as they are: easi.run, Chrome, MCP, API, SDK, token (may be translated if the language has a common
equivalent in software), proxy, WebM, HTTPS, URL, Developer tools names, product and file names, and anything inside
backticks. Never translate {placeholders}: keep each one exactly, once, where the sentence needs it. Match the English's
length and tone: short labels stay short (a button is one or two words), sentences stay one sentence. Use the
language's usual typography (quotes, ellipsis, non-breaking spaces where usual).
A message is either a text or a plural message: an object of CLDR plural categories. For a plural message, answer with
exactly the categories you are given for the language, each a full text; {count} is the number.
Answer with one JSON object only, no prose: each key you were given, its translation.`;

/** The review of a batch's translations: their meaning against English's, nothing else. */
const REVIEW = `${BRIEF}
You review translations of these messages. For each key, compare the translation with the English and report only a
real problem of meaning: a claim added, removed, strengthened or weakened (a source that says nothing turned into one
that says something, "not documented" into a statement); a changed number, price, unit, limit, plan or date; a number
written in a way that means another value in that language (a thousands separator read as a decimal mark there); a
narrowed or widened scope; a different subject, object or trigger. Ignore style, word choice and length.
Answer with one JSON object: each key with a problem, its problem in one line of English. Answer {} when there is none.`;

/** What `ask` answers when the answer was cut at its length. */
const TRUNCATED = Symbol('truncated');

/** Batches translated at once per language. */
const CONCURRENCY = 6;

/** Short labels the language's catalog has already, those of the areas `keys` are in first, at most `GLOSSARY`, as
 * English and translation: the words a new message uses for the same things (a session, a proxy), so a batch does
 * not name them otherwise. None of `pending`: their translations are of an English that changed, or none yet. */
const GLOSSARY = 80;
function glossary(english, catalog, keys, pending) {
  const areas = new Set(keys.map((k) => k.slice(0, k.indexOf('.'))));
  const known = Object.keys(catalog).filter(
    (k) => !pending.has(k) && typeof catalog[k] === 'string' && typeof english[k] === 'string' && english[k].length <= 48,
  );
  const ordered = [...known.filter((k) => areas.has(k.slice(0, k.indexOf('.')))), ...known.filter((k) => !areas.has(k.slice(0, k.indexOf('.'))))];
  return Object.fromEntries(ordered.slice(0, GLOSSARY).map((k) => [english[k], catalog[k]]));
}

/** `pending` English messages translated into `locale` by Claude, checked, in batches run side by side; `done` takes
 * each batch as it ends; `catalog` is what the language has already (its words for things, see `glossary`). Throws,
 * once every batch ended, if any answer stayed unusable. */
async function translate(locale, english, pending, key, done, catalog) {
  const categories = new Intl.PluralRules(locale).resolvedOptions().pluralCategories;
  const waiting = new Set(pending);
  const batches = [];
  for (let at = 0; at < pending.length; at += BATCH) {
    batches.push(Object.fromEntries(pending.slice(at, at + BATCH).map((k) => [k, english[k]])));
  }
  /** Claude's JSON answer to `user` under `system`, or null when the answer is not one JSON object. */
  const ask = async (system, user) => {
    const response = await fetch('https://api.anthropic.com/v1/messages', {
      method: 'POST',
      headers: { 'x-api-key': key, 'anthropic-version': '2023-06-01', 'content-type': 'application/json' },
      body: JSON.stringify({ model: MODEL, max_tokens: 16000, system, messages: [{ role: 'user', content: user }] }),
    });
    if (!response.ok) throw new Error(`${locale}: the translator answered ${response.status}: ${(await response.text()).slice(0, 300)}`);
    const body = await response.json();
    // An answer cut at its length: too much asked at once (see `one`).
    if (body.stop_reason === 'max_tokens') return TRUNCATED;
    const answer = body.content.map((c) => c.text ?? '').join('');
    try {
      const parsed = JSON.parse(answer.slice(answer.indexOf('{'), answer.lastIndexOf('}') + 1));
      return parsed !== null && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : null;
    } catch {
      return null;
    }
  };
  /** The review of `pairs`: each key with a problem, its problem. Only a valid answer counts (an empty one approves);
   * one that stays invalid fails the batch. */
  const review = async (pairs) => {
    for (let attempt = 0; attempt < 3; attempt++) {
      const found = await ask(REVIEW, `Language: ${NAMES[locale]} (${locale}).\nMessages:\n${JSON.stringify(pairs, null, 1)}`);
      if (typeof found === 'object' && found !== null && Object.entries(found).every(([k, why]) => Object.hasOwn(pairs, k) && typeof why === 'string')) return found;
    }
    throw new Error(`${locale}: the review's answer stayed unusable`);
  };
  // Each batch is translated, checked (placeholders, plural forms), then reviewed against English for its meaning;
  // what the review finds is translated again, told why, up to two more times.
  const one = async (batch) => {
    // Long messages in a language that takes many tokens may not fit one answer: then each half on its own.
    const halves = async (some) => {
      const keys = Object.keys(some);
      const half = Math.ceil(keys.length / 2);
      await one(Object.fromEntries(keys.slice(0, half).map((k) => [k, some[k]])));
      await one(Object.fromEntries(keys.slice(half).map((k) => [k, some[k]])));
    };
    let todo = batch;
    let feedback = '';
    const accepted = {};
    const known = glossary(english, catalog, Object.keys(batch), waiting);
    // Pairs, not an object keyed by English: an answer must hold the batch's keys alone, and a glossary shaped like a
    // batch was answered as one.
    const terms = Object.keys(known).length
      ? `For reference only, not to translate: labels this interface already has in this language, each as [English, translation]. Name the same things with the same words:\n${JSON.stringify(Object.entries(known))}\nAnswer only the messages below, each by its key.\n`
      : '';
    for (let attempt = 0; ; attempt++) {
      const parsed = await ask(SYSTEM, `Language: ${NAMES[locale]} (${locale}); ${REGISTER[locale]}. Plural categories for this language: ${categories.join(', ')}.\n${terms}${feedback}Messages:\n${JSON.stringify(todo, null, 1)}`);
      if (parsed === TRUNCATED) {
        // What was accepted so far is kept; what is left goes in halves.
        if (Object.keys(accepted).length) await done(accepted);
        if (Object.keys(todo).length < 2) throw new Error(`${locale}: an answer too long for one message`);
        return halves(todo);
      }
      const errors = [];
      if (parsed === null) errors.push('the answer is not one JSON object');
      else {
        for (const k of Object.keys(parsed)) parsed[k] = trimmed(parsed[k], locale);
        for (const k of Object.keys(todo)) {
          if (!(k in parsed)) errors.push(`${k}: missing`);
          else {
            const p = problem(todo[k], parsed[k], locale);
            if (p) errors.push(`${k}: ${p}`);
          }
        }
        for (const k of Object.keys(parsed)) if (!(k in todo)) errors.push(`${k}: not asked for`);
      }
      if (errors.length) {
        if (attempt === 2) throw new Error(`${locale}: unusable translations:\n  ${errors.join('\n  ')}`);
        feedback = `Your previous answer had these problems; fix them:\n${errors.join('\n')}\n`;
        continue;
      }
      const pairs = Object.fromEntries(Object.keys(todo).map((k) => [k, { english: todo[k], translation: parsed[k] }]));
      const found = await review(pairs);
      const flagged = Object.keys(found);
      for (const k of Object.keys(todo)) if (!flagged.includes(k) || attempt === 2) accepted[k] = parsed[k];
      if (flagged.length === 0 || attempt === 2) {
        // Kept after three tries: said, for a person to look at.
        if (flagged.length) console.warn(`${locale}: kept despite the review:\n  ${flagged.map((k) => `${k}: ${found[k]}`).join('\n  ')}`);
        return done(accepted);
      }
      todo = Object.fromEntries(flagged.map((k) => [k, batch[k]]));
      feedback = `A review of your previous translations of these messages found these problems; translate them again, fixing them:\n${flagged.map((k) => `${k}: ${found[k]}`).join('\n')}\n`;
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
if (Object.keys(english).length === 0) throw new Error(`${MESSAGES}/en has no messages`);
for (const [key, message] of Object.entries(english)) {
  if (typeof message === 'string' && isRich(message)) {
    try {
      parseRich(message);
    } catch (e) {
      throw new Error(`en: ${key}: ${e.message}`);
    }
  }
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
  if (currentTypes !== types(english)) failures.push('src/i18n/keys.ts is not current: run node ../i18n/sync.mjs');
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
        if (redo.length) await translate(p.locale, english, redo, key, done, p.catalog);
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
