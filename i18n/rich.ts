// A message with inline markup (the docs' prose): text with {placeholders}, and a closed set of elements, <code> (its
// text literal: no placeholder, no element), <strong>, <em> and <a href="…">, nested properly (no link in a link).
// Only &lt; &gt; &amp; and &quot; are entities (a & that starts none is itself). The same parser renders a message (Rich.svelte) and checks a
// translation against its English (sync.mjs), so a translation that would render differently, or not at all, is
// refused before it is kept. A message is parsed before any parameter goes in, and parameters are text: a value is
// never read as markup.

import { PLACEHOLDER_SOURCE } from './format.ts';

export type RichNode =
  | { kind: 'text'; text: string }
  | { kind: 'param'; name: string }
  | { kind: 'code'; text: string }
  | { kind: 'strong' | 'em'; children: RichNode[] }
  | { kind: 'a'; href: string; children: RichNode[] };

const PARAM = new RegExp(`^${PLACEHOLDER_SOURCE}`);
const ENTITIES: Record<string, string> = { '&lt;': '<', '&gt;': '>', '&amp;': '&', '&quot;': '"' };
const decode = (text: string, where: string) =>
  text.replace(/&(?:[a-z]+|#[0-9]+);/g, (entity) => {
    const decoded = ENTITIES[entity];
    if (decoded === undefined) throw new Error(`${where}: unknown entity ${entity}`);
    return decoded;
  });

/** Parses `message`; throws, saying where, on anything outside the grammar. */
export function parseRich(message: string): RichNode[] {
  let at = 0;
  const fail = (why: string): never => {
    throw new Error(`${why} at ${at}: ${JSON.stringify(message.slice(Math.max(0, at - 20), at + 20))}`);
  };
  function nodes(closing: string | null, inLink: boolean): RichNode[] {
    const out: RichNode[] = [];
    let text = '';
    const flush = () => {
      if (text) out.push({ kind: 'text', text: decode(text, `text before ${at}`) });
      text = '';
    };
    while (at < message.length) {
      const rest = message.slice(at);
      if (closing && rest.startsWith(`</${closing}>`)) {
        flush();
        at += closing.length + 3;
        return out;
      }
      if (rest.startsWith('{')) {
        const m = PARAM.exec(rest);
        if (!m) fail('a { that opens no placeholder');
        flush();
        out.push({ kind: 'param', name: m![1]! });
        at += m![0].length;
        continue;
      }
      if (rest.startsWith('}')) fail('a } that closes no placeholder');
      if (rest.startsWith('<')) {
        flush();
        if (rest.startsWith('<code>')) {
          const end = message.indexOf('</code>', at + 6);
          if (end < 0) fail('an unclosed <code>');
          const raw = message.slice(at + 6, end);
          if (raw.includes('<')) fail('an element in <code>');
          out.push({ kind: 'code', text: decode(raw, `code at ${at}`) });
          at = end + 7;
        } else if (rest.startsWith('<strong>') || rest.startsWith('<em>')) {
          const kind = rest.startsWith('<strong>') ? 'strong' : 'em';
          at += kind.length + 2;
          out.push({ kind, children: nodes(kind, inLink) });
        } else if (rest.startsWith('<a href="')) {
          if (inLink) fail('a link in a link');
          const end = message.indexOf('">', at + 9);
          if (end < 0) fail('an unclosed <a href');
          const href = decode(message.slice(at + 9, end), `href at ${at}`);
          if (/["<>\s]/.test(href) || href === '') fail('a malformed href');
          at = end + 2;
          out.push({ kind: 'a', href, children: nodes('a', true) });
        } else fail('an element outside <code>, <strong>, <em> and <a href="…">');
        continue;
      }
      if (rest.startsWith('>')) fail('a stray >');
      text += message[at];
      at += 1;
    }
    if (closing) fail(`an unclosed <${closing}>`);
    flush();
    return out;
  }
  return nodes(null, false);
}

/** Every placeholder's name in `nodes`, outside <code>, as many times as each is there. */
export function richParams(nodes: RichNode[]): string[] {
  return nodes.flatMap((n) => (n.kind === 'param' ? [n.name] : 'children' in n ? richParams(n.children) : [])).sort();
}

/** What a translation must keep of its English, each sorted so its parts may move: every <code>'s text, every link
 * with its href and the code in it, how many <strong> and <em> there are, and the placeholders. */
export function richShape(nodes: RichNode[]): string {
  const codes: string[] = [];
  const links: string[] = [];
  const marks = { strong: 0, em: 0 };
  const walk = (list: RichNode[]) => {
    for (const n of list) {
      if (n.kind === 'code') codes.push(n.text);
      else if (n.kind === 'a') {
        const inner: string[] = [];
        const codeIn = (l: RichNode[]) => l.forEach((c) => (c.kind === 'code' ? inner.push(c.text) : 'children' in c && codeIn(c.children)));
        codeIn(n.children);
        links.push(`${n.href} ${JSON.stringify(inner.sort())}`);
        walk(n.children);
      } else if (n.kind === 'strong' || n.kind === 'em') {
        marks[n.kind] += 1;
        walk(n.children);
      }
    }
  };
  walk(nodes);
  return JSON.stringify({ codes: codes.sort(), links: links.sort(), marks, params: richParams(nodes) });
}

/** Whether `message` uses any of the markup (a plain text is checked as a plain text). */
export const isRich = (message: string) => /<(code|strong|em)>|<a href="/.test(message);
