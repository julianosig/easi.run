<script lang="ts">
  // A choice among many, by typing: a field (its icon inside, at its start) that filters the options as letters go in,
  // the matches listed under it. Arrow keys move through them, Enter or a click picks one, Escape closes the list; the
  // field then shows the pick. Announced as a combobox with its listbox (WAI-ARIA's pattern, the list's active
  // option followed by the field's aria-activedescendant).
  import Icon from './Icon.svelte';
  import { field } from './ui.ts';

  type Option = { value: string; label: string };
  let {
    id,
    label,
    icon,
    placeholder,
    options,
    value,
    empty,
    describedby,
    ref = $bindable(),
    onpick,
  }: {
    id: string;
    /** Its name, for screen readers (the icon and placeholder say it on screen). */
    label: string;
    icon: string;
    placeholder: string;
    options: Option[];
    /** The option picked, if any. */
    value: string;
    /** What the list says when nothing matches. */
    empty: string;
    describedby?: string | undefined;
    ref?: HTMLInputElement | undefined;
    onpick: (value: string) => void;
  } = $props();

  const picked = $derived(options.find((o) => o.value === value)?.label ?? '');
  /** What is typed while searching; null shows the pick. */
  let query: string | null = $state(null);
  let open = $state(false);
  let active = $state(0);
  const matches = $derived.by(() => {
    const q = (query ?? '').trim().toLocaleLowerCase();
    return q === '' ? options : options.filter((o) => o.label.toLocaleLowerCase().includes(q));
  });
  const listId = $derived(`${id}-list`);
  const optionId = (i: number) => `${id}-option-${i}`;

  // The active option kept in sight of the bounded list, as the arrows move it or typing changes the matches.
  $effect(() => {
    if (open && matches.length) document.getElementById(optionId(active))?.scrollIntoView({ block: 'nearest' });
  });

  function show() {
    open = true;
    const at = matches.findIndex((o) => o.value === value);
    active = at < 0 ? 0 : at;
  }
  function pick(option: Option) {
    query = null;
    open = false;
    onpick(option.value);
  }
  function close() {
    query = null;
    open = false;
  }
  function key(event: KeyboardEvent) {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      if (!open) return show();
      const n = matches.length;
      if (n) active = (active + (event.key === 'ArrowDown' ? 1 : n - 1)) % n;
    } else if (event.key === 'Enter' && open) {
      event.preventDefault();
      const option = matches[active];
      if (option) pick(option);
    } else if (event.key === 'Escape' && open) {
      // The list closes; the dialog around it stays.
      event.preventDefault();
      event.stopPropagation();
      close();
    }
  }
</script>

<div class="relative">
  <Icon d={icon} class="pointer-events-none absolute left-4 top-1/2 size-[18px] -translate-y-1/2 {open ? 'text-accent' : 'text-muted'}" />
  <input
    {id}
    bind:this={ref}
    class="{field} pl-11"
    type="text"
    role="combobox"
    aria-label={label}
    aria-autocomplete="list"
    aria-expanded={open}
    aria-controls={listId}
    aria-activedescendant={open && matches[active] ? optionId(active) : undefined}
    aria-describedby={describedby}
    autocomplete="off"
    spellcheck="false"
    {placeholder}
    value={query ?? picked}
    onfocus={(e) => (e.currentTarget.select(), show())}
    onclick={() => open || show()}
    oninput={(e) => ((query = e.currentTarget.value), (open = true), (active = 0))}
    onkeydown={key}
    onblur={close}
  />
  {#if open}
    <ul
      id={listId}
      role="listbox"
      aria-label={label}
      class="absolute inset-x-0 top-full z-20 mt-1.5 max-h-56 overflow-y-auto rounded-xl border border-line bg-paper p-1 shadow-[0_18px_40px_-18px_rgb(20_40_75/0.45)]"
    >
      {#each matches as option, i (option.value)}
        <!-- Picked on the press, before the field's blur would close the list. -->
        <li
          id={optionId(i)}
          role="option"
          aria-selected={option.value === value}
          class="flex cursor-pointer items-center justify-between rounded-lg px-3 py-2 text-[15px] {i === active ? 'bg-accent-soft text-accent' : 'text-ink'}"
          onmousedown={(e) => (e.preventDefault(), pick(option))}
          onmousemove={() => (active = i)}
        >
          {option.label}
          {#if option.value === value}<span class="size-1.5 rounded-full bg-accent" aria-hidden="true"></span>{/if}
        </li>
      {:else}
        <li class="px-3 py-2 text-sm text-muted" role="presentation">{empty}</li>
      {/each}
    </ul>
  {/if}
</div>
