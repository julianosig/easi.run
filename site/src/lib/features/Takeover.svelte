<script lang="ts">
  // The agents' card, drawn: the blue cursor (your agent) reads a page until a login appears, then waits aside while the
  // coral one (you) types the password; a check, and the agent carries on to the button. Decorative. It loops every 7 s,
  // moving only transforms and opacity; for people who ask for less motion it stands still on the typed field, both
  // cursors on the page (app.css stops every animation, and what is left is the frame each element is drawn at).
  const ARROW = 'M12 6 56 30 35 36 25 58Z';
</script>

{#snippet cursor(role: string, fill: string)}
  <!-- Its tip is the group's origin. -->
  <g class={role}><path d={ARROW} transform="scale(0.42) translate(-12 -6)" class="{fill} stroke-white" stroke-width="5" stroke-linejoin="round" /></g>
{/snippet}

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The window: its toolbar, then the page, a few lines of text on the left. -->
  <path d="M34 6h252a10 10 0 0 1 10 10v8H24v-8a10 10 0 0 1 10-10z" class="fill-accent-soft" />
  <rect x="24" y="24" width="272" height="98" class="fill-paper" />
  <rect x="24" y="6" width="272" height="116" rx="10" class="fill-none stroke-line" stroke-width="1.5" />
  <circle cx="36" cy="15" r="2.5" class="fill-white" /><circle cx="44" cy="15" r="2.5" class="fill-white" /><circle cx="52" cy="15" r="2.5" class="fill-white" />
  <rect x="64" y="10.5" width="112" height="9" rx="4.5" class="fill-paper" />
  <rect x="40" y="38" width="64" height="8" rx="4" class="fill-ink/15" />
  <rect x="40" y="54" width="100" height="5" rx="2.5" class="fill-wash" />
  <rect x="40" y="65" width="86" height="5" rx="2.5" class="fill-wash" />
  <rect x="40" y="76" width="96" height="5" rx="2.5" class="fill-wash" />
  <rect x="40" y="87" width="58" height="5" rx="2.5" class="fill-wash" />

  <!-- The login: a label, the password field with its lock, the dots you type, the check, the button. -->
  <g class="form">
    <rect x="172" y="42" width="40" height="5" rx="2.5" class="fill-ink/15" />
    <rect x="172" y="54" width="108" height="24" rx="6" class="fill-paper stroke-accent" stroke-width="1.5" />
    <path d="M181.5 65v-2.5a3 3 0 0 1 6 0V65" class="fill-none stroke-muted" stroke-width="1.6" />
    <rect x="179" y="64.5" width="11" height="8" rx="1.5" class="fill-muted" />
    {#each [0, 1, 2, 3, 4, 5] as i (i)}
      <circle cx={202 + i * 9} cy="66" r="2.6" class="dot d{i} fill-ink" />
    {/each}
    <g class="check">
      <circle cx="267" cy="66" r="7" class="fill-accent" />
      <path d="m263.5 66 2.5 2.5 4.5-5" class="fill-none stroke-white" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
    </g>
    <rect x="172" y="88" width="52" height="14" rx="7" class="fill-accent" />
  </g>

  {@render cursor('agent', 'fill-accent')}
  {@render cursor('you', 'fill-you')}
</svg>

<style>
  /* The still frame: the field typed and checked, you on it, the agent waiting aside. */
  .agent {
    transform: translate(124px, 100px);
    animation: agent 7s ease-in-out infinite;
  }
  .you {
    transform: translate(198px, 70px);
    animation: you 7s ease-in-out infinite;
  }
  .form {
    animation: form 7s ease-out infinite;
  }
  .dot {
    animation: dot 7s step-end infinite both;
  }
  /* Typed one after another, 0.17 s apart: in classes, as the page's policy allows no inline style. */
  .d1 {
    animation-delay: 0.17s;
  }
  .d2 {
    animation-delay: 0.34s;
  }
  .d3 {
    animation-delay: 0.51s;
  }
  .d4 {
    animation-delay: 0.68s;
  }
  .d5 {
    animation-delay: 0.85s;
  }
  .check {
    transform-box: fill-box;
    transform-origin: center;
    animation: check 7s ease-out infinite;
  }

  @keyframes agent {
    0% {
      transform: translate(48px, 36px);
      opacity: 0;
    }
    4% {
      opacity: 1;
    }
    13% {
      transform: translate(132px, 54px);
    }
    21% {
      transform: translate(92px, 72px);
    }
    31%,
    68% {
      transform: translate(124px, 100px);
    }
    34%,
    66% {
      opacity: 0.55;
    }
    70% {
      opacity: 1;
    }
    80% {
      transform: translate(204px, 94px) scale(1);
    }
    83% {
      transform: translate(204px, 94px) scale(0.82);
    }
    86%,
    92% {
      transform: translate(204px, 94px) scale(1);
      opacity: 1;
    }
    96% {
      transform: translate(204px, 94px);
      opacity: 0;
    }
    100% {
      transform: translate(48px, 36px);
      opacity: 0;
    }
  }
  @keyframes you {
    0%,
    30% {
      transform: translate(330px, 136px);
      opacity: 0;
    }
    33% {
      opacity: 1;
    }
    43%,
    63% {
      transform: translate(198px, 70px);
    }
    72% {
      opacity: 1;
    }
    76%,
    100% {
      transform: translate(330px, 136px);
      opacity: 0;
    }
  }
  @keyframes form {
    0%,
    18% {
      transform: translateY(5px);
      opacity: 0;
    }
    25%,
    92% {
      transform: none;
      opacity: 1;
    }
    97%,
    100% {
      opacity: 0;
    }
  }
  @keyframes dot {
    0% {
      opacity: 0;
    }
    45% {
      opacity: 1;
    }
    92% {
      opacity: 0;
    }
  }
  @keyframes check {
    0%,
    61% {
      transform: scale(0);
      opacity: 0;
    }
    65% {
      transform: scale(1.15);
      opacity: 1;
    }
    68%,
    92% {
      transform: scale(1);
      opacity: 1;
    }
    97%,
    100% {
      opacity: 0;
    }
  }
</style>
