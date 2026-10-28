<script lang="ts">
  // A message with inline markup (i18n/rich.ts), drawn as elements: never as HTML. The message is parsed before its
  // parameters go in, and they go in as text; a link's address comes from the message (as written in English) or a
  // parameter, and only a page here (#/…), a path on the site or an https:// address is followed.
  import { parseRich, type RichNode } from '../../../i18n/rich.ts';
  import { localized, template } from './i18n.ts';

  let { message, params = {} }: { message: string; params?: Record<string, string> } = $props();

  const nodes = $derived.by((): RichNode[] => {
    try {
      return parseRich(template(message));
    } catch {
      return [{ kind: 'text', text: message }];
    }
  });
  const value = (name: string) => params[name] ?? `{${name}}`;
  function href(raw: string): string {
    const named = /^\{([a-zA-Z][a-zA-Z0-9_]*)\}$/.exec(raw);
    const url = named ? value(named[1]!) : raw;
    return /^(#\/|\/(?!\/)|https:\/\/)/.test(url) ? localized(url) : '#';
  }
</script>

{#snippet draw(list: RichNode[])}{#each list as node, i (i)}{#if node.kind === 'text'}{node.text}{:else if node.kind === 'param'}{value(node.name)}{:else if node.kind === 'code'}<code>{node.text}</code>{:else if node.kind === 'strong'}<strong>{@render draw(node.children)}</strong>{:else if node.kind === 'em'}<em>{@render draw(node.children)}</em>{:else if node.kind === 'a'}<a href={href(node.href)}>{@render draw(node.children)}</a>{/if}{/each}{/snippet}
{@render draw(nodes)}
