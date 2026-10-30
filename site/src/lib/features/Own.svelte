<script lang="ts">
  // Each agent its own, drawn: an agent's token in the middle, linked to its own sessions on the left, which light up;
  // on the right your settings, your other tokens and the live view stay shut, each behind a lock that pulses when the
  // agent turns that way. Decorative. It loops every 8 s, moving only transforms and opacity; for less motion it stands
  // still with its sessions linked and the rest locked.
  import Cursor from './Cursor.svelte';
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- Its own sessions. -->
  {#each [22, 70] as y (y)}
    <rect x="20" y={y} width="84" height="36" rx="7" class="fill-paper stroke-line" stroke-width="1.5" />
    <path d="M27 {y}h70a7 7 0 0 1 7 7v1H20v-1a7 7 0 0 1 7-7z" class="fill-accent-soft" />
    <rect x="28" y={y + 16} width="44" height="4" rx="2" class="fill-ink/15" />
    <rect x="28" y={y + 25} width="60" height="4" rx="2" class="fill-wash" />
  {/each}
  <path d="M138 64h-20l-14-24M118 64l-14 24" class="link fill-none stroke-accent" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />

  <!-- The agent, its key beside it. -->
  <circle cx="160" cy="64" r="20" class="fill-accent-soft" />
  <g transform="translate(156 56)"><Cursor /></g>

  <!-- What it never reaches: settings, tokens, the live view, each locked. -->
  {#each [{ y: 18, d: 'M0 -5v10M-5 0h10' }, { y: 52, d: 'M-4 0a4 4 0 1 0 8 0a4 4 0 1 0 -8 0M4 0h4' }, { y: 86, d: 'M-6 0q6 -6 12 0q-6 6 -12 0' }] as row, i (row.y)}
    <rect x="212" y={row.y} width="88" height="24" rx="7" class="fill-wash" />
    <g transform="translate(232 {row.y + 12})"><path d={row.d} class="fill-none stroke-muted/60" stroke-width="1.8" stroke-linecap="round" /></g>
    <rect x="246" y={row.y + 10} width="28" height="4" rx="2" class="fill-ink/10" />
    <g class="lock l{i}" transform="translate(288 {row.y + 12})">
      <path d="M-3.5 -1v-2.5a3.5 3.5 0 0 1 7 0V-1" class="fill-none stroke-ink" stroke-width="1.6" />
      <rect x="-5" y="-1.5" width="10" height="7.5" rx="1.5" class="fill-ink" />
    </g>
  {/each}
</svg>

<style>
  .link {
    animation: link 8s ease-in-out infinite;
  }
  .lock > * {
    transform-box: view-box;
  }
  .lock {
    animation: lock 8s ease-in-out infinite;
  }
  .l1 {
    animation-delay: 0.5s;
  }
  .l2 {
    animation-delay: 1s;
  }

  @keyframes link {
    0%,
    8% {
      opacity: 0;
    }
    20%,
    86% {
      opacity: 1;
    }
    96%,
    100% {
      opacity: 0;
    }
  }
  @keyframes lock {
    0%,
    44% {
      opacity: 1;
    }
    50% {
      opacity: 0.35;
    }
    56%,
    100% {
      opacity: 1;
    }
  }
</style>
