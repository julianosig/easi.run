// Syntax highlighting for the docs' code blocks, at prerender as in the browser: highlight.js's core with only the
// languages the docs show, so the bundle carries no other grammar. A block in any other language (a prompt, a proxy's
// URL) stays plain text.
import hljs from 'highlight.js/lib/core';
import bash from 'highlight.js/lib/languages/bash';
import http from 'highlight.js/lib/languages/http';
import ini from 'highlight.js/lib/languages/ini';
import javascript from 'highlight.js/lib/languages/javascript';
import xml from 'highlight.js/lib/languages/xml';
import yaml from 'highlight.js/lib/languages/yaml';

/** Each block's `lang`, as the docs name it, and the grammar that reads it. */
const GRAMMARS = { shell: bash, http, ini, '.env': ini, javascript, html: xml, yaml } as const;
for (const [name, grammar] of Object.entries(GRAMMARS)) hljs.registerLanguage(name, grammar);

/** `code` as HTML with its tokens in spans (highlight.js escapes the code itself), or null for a language it lacks. */
export function highlight(code: string, lang: string): string | null {
  return lang in GRAMMARS ? hljs.highlight(code, { language: lang, ignoreIllegals: true }).value : null;
}
