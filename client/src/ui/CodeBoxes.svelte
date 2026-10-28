<script lang="ts">
  // A 6-digit code as six boxes, one digit each. One real field lies over them, unseen, so typing, pasting, a password
  // manager's or the phone's fill and a screen reader all meet a single field; the boxes only show its digits and where
  // the next one goes. The sixth digit hands the code on (`oncomplete`).
  let {
    id,
    value = $bindable(''),
    disabled = false,
    ref = $bindable(),
    oncomplete,
  }: { id: string; value?: string; disabled?: boolean; ref?: HTMLInputElement | undefined; oncomplete: (code: string) => void } = $props();
  let focused = $state(false);

  // No maxlength: a code pasted with a space or dash (492 817) keeps its six digits, the rest taken out here.
  function typed(event: Event & { currentTarget: HTMLInputElement }) {
    const input = event.currentTarget;
    const digits = input.value.replace(/\D/g, '').slice(0, 6);
    input.value = digits;
    value = digits;
    if (digits.length === 6) oncomplete(digits);
  }
  // The next digit always goes at the end, whatever was clicked or selected.
  const toEnd = (event: Event & { currentTarget: HTMLInputElement }) => {
    const end = event.currentTarget.value.length;
    event.currentTarget.setSelectionRange(end, end);
  };
</script>

<div class="relative grid max-w-sm grid-cols-6 gap-2 sm:gap-2.5">
  {#each { length: 6 } as _, i (i)}
    {@const next = focused && i === Math.min(value.length, 5)}
    <span
      class="grid aspect-[5/6] place-items-center rounded-xl border bg-paper font-mono text-2xl font-semibold text-ink transition {next
        ? 'border-accent ring-4 ring-accent/15'
        : 'border-line'} {disabled ? 'opacity-50' : ''}"
      aria-hidden="true"
    >
      {#if value[i]}{value[i]}{:else if next}<span class="caret h-7 w-0.5 rounded-full bg-accent"></span>{/if}
    </span>
  {/each}
  <input
    {id}
    bind:this={ref}
    class="absolute inset-0 size-full cursor-text opacity-0"
    type="text"
    inputmode="numeric"
    autocomplete="one-time-code"
    pattern="[0-9]*"
    spellcheck="false"
    {disabled}
    {value}
    oninput={typed}
    onfocus={(e) => ((focused = true), toEnd(e))}
    onblur={() => (focused = false)}
    onclick={toEnd}
  />
</div>

<style>
  .caret {
    animation: blink 1.1s steps(1) infinite;
  }
  @keyframes blink {
    50% {
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .caret {
      animation: none;
    }
  }
</style>
