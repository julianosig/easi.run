<script lang="ts">
  // The timeline and its replay, drawn: on the left, what happened, one event under another (the agent's in blue, yours
  // in coral, a problem in amber); on the right, the replay of the agent's screen, its pointer at work, and under it the
  // position bar with a mark per event. The playhead runs along the bar, and the event it passes lights up on the left.
  // Decorative. It loops every 8 s, moving only transforms and opacity; for less motion it stands still on the third
  // event.
  import Cursor from './Cursor.svelte';
  import Window from './Window.svelte';

  /** Each event: its dot's colour and, on the position bar, its mark's x. */
  const EVENTS = [
    { fill: 'fill-accent', x: 172 },
    { fill: 'fill-accent', x: 196 },
    { fill: 'fill-partly', x: 226 },
    { fill: 'fill-you', x: 250 },
    { fill: 'fill-accent', x: 274 },
  ];
</script>

<Window>
  <!-- The event the playhead last passed, behind its line. -->
  <rect x="32" y="32" width="120" height="14" rx="5" class="now fill-accent-soft" />
  {#each EVENTS as event, i (i)}
    <circle cx="42" cy={39 + i * 17} r="3.5" class={event.fill} />
    <rect x="52" y={36.5 + i * 17} width={[72, 88, 60, 80, 68][i]} height="5" rx="2.5" class="fill-ink/15" />
  {/each}

  <!-- The replay: the agent's screen, its pointer, the play mark. -->
  <rect x="164" y="32" width="120" height="66" rx="8" class="fill-wash" />
  <rect x="174" y="42" width="48" height="6" rx="3" class="fill-ink/15" />
  <rect x="174" y="54" width="92" height="4" rx="2" class="fill-paper" />
  <rect x="174" y="63" width="80" height="4" rx="2" class="fill-paper" />
  <rect x="174" y="76" width="40" height="12" rx="6" class="fill-accent/70" />
  <g class="agent"><Cursor /></g>

  <!-- The position bar, a mark per event, and the playhead. -->
  <rect x="164" y="106" width="120" height="4" rx="2" class="fill-line" />
  {#each EVENTS as event, i (i)}
    <circle cx={event.x} cy="108" r="2.2" class={event.fill} />
  {/each}
  <g class="head">
    <rect x="163" y="101" width="2" height="14" rx="1" class="fill-ink" />
  </g>
</Window>

<style>
  /* The still frame: the playhead on the third mark, the third event lit. */
  .head {
    transform: translateX(62px);
    animation: head 8s linear infinite;
  }
  .now {
    transform: translateY(34px);
    animation: now 8s step-end infinite;
  }
  .agent {
    transform: translate(204px, 82px);
    animation: agent 8s ease-in-out infinite;
  }

  @keyframes head {
    from {
      transform: translateX(0);
    }
    to {
      transform: translateX(120px);
    }
  }
  /* Each event lights as the playhead reaches its mark: at 8/120, 32/120, 62/120, 86/120 and 110/120 of the loop. */
  @keyframes now {
    0% {
      transform: translateY(0);
      opacity: 0;
    }
    6.7% {
      transform: translateY(0);
      opacity: 1;
    }
    26.7% {
      transform: translateY(17px);
      opacity: 1;
    }
    51.7% {
      transform: translateY(34px);
      opacity: 1;
    }
    71.7% {
      transform: translateY(51px);
      opacity: 1;
    }
    91.7% {
      transform: translateY(68px);
      opacity: 1;
    }
  }
  @keyframes agent {
    0%,
    100% {
      transform: translate(186px, 50px);
    }
    25% {
      transform: translate(240px, 58px);
    }
    50% {
      transform: translate(204px, 82px);
    }
    75% {
      transform: translate(250px, 66px);
    }
  }
</style>
