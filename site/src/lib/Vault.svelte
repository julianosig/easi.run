<script lang="ts">
  // Your card, drawn: a logged-in session (a check and a key) drops into a vault, whose door swings shut; a padlock
  // clicks closed on it and sends out a soft pulse. Decorative. It loops every 7 s, moving only transforms and opacity;
  // for people who ask for less motion it stands still on the locked vault (app.css stops every animation, and what is
  // left is the frame each element is drawn at).
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The vault, centred: its body on two feet, and the dark of its inside. -->
  <rect x="117" y="118" width="14" height="6" rx="2" class="fill-ink" />
  <rect x="189" y="118" width="14" height="6" rx="2" class="fill-ink" />
  <rect x="103" y="8" width="114" height="112" rx="14" class="fill-paper stroke-ink" stroke-width="2.5" />
  <rect x="113" y="18" width="94" height="92" rx="8" class="fill-wash" />

  <!-- The session: logged in (the check), with its key; drawn where it rests inside, and dropped in from above. -->
  <g class="session">
    <rect x="120" y="40" width="80" height="48" rx="8" class="fill-paper stroke-line" stroke-width="1.5" />
    <circle cx="140" cy="64" r="9" class="fill-accent" />
    <path d="m136 64 3 3 5-6" class="fill-none stroke-white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
    <circle cx="164" cy="64" r="5" class="fill-none stroke-ink" stroke-width="2.4" />
    <path d="M169 64h18m-4 0v5m4-5v4" class="fill-none stroke-ink" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" />
  </g>

  <!-- The door, hinged on the right: drawn over the session, so it hides it once shut. -->
  <g class="door">
    <rect x="113" y="18" width="94" height="92" rx="8" class="fill-paper stroke-ink" stroke-width="2" />
    <rect x="121" y="56" width="4" height="16" rx="2" class="fill-ink/25" />
  </g>

  <circle cx="160" cy="66" r="22" class="pulse fill-none stroke-accent" stroke-width="2" />
  <g class="lock">
    <path d="M152 60v-6a8 8 0 0 1 16 0v6" class="shackle fill-none stroke-ink" stroke-width="3.5" />
    <rect x="148" y="59" width="24" height="19" rx="4" class="fill-ink" />
    <circle cx="160" cy="67" r="2.4" class="fill-paper" />
    <rect x="159" y="68" width="2" height="5" rx="1" class="fill-paper" />
  </g>
</svg>

<style>
  /* The still frame: the door shut, the padlock locked, the session inside. */
  .session {
    opacity: 0;
    animation: session 7s ease-in-out infinite;
  }
  .door {
    transform-box: fill-box;
    transform-origin: 100% 50%;
    animation: door 7s ease-in-out infinite;
  }
  .lock {
    transform-box: fill-box;
    transform-origin: center;
    animation: lock 7s ease-out infinite;
  }
  .shackle {
    animation: shackle 7s ease-in infinite;
  }
  .pulse {
    transform-box: fill-box;
    transform-origin: center;
    opacity: 0;
    animation: pulse 7s ease-out infinite;
  }

  @keyframes session {
    0% {
      transform: translateY(-58px);
      opacity: 0;
    }
    8% {
      transform: translateY(-58px);
      opacity: 1;
    }
    30%,
    86% {
      transform: none;
      opacity: 1;
    }
    87%,
    100% {
      transform: translateY(-58px);
      opacity: 0;
    }
  }
  /* Open, the door is a sliver at its hinge, as seen edge-on; shut, it is whole. Never mirrored. */
  @keyframes door {
    0%,
    30% {
      transform: scaleX(0.08);
    }
    46%,
    88% {
      transform: none;
    }
    98%,
    100% {
      transform: scaleX(0.08);
    }
  }
  @keyframes lock {
    0%,
    50% {
      transform: none;
      opacity: 0;
    }
    54% {
      opacity: 1;
    }
    62% {
      transform: none;
    }
    64% {
      transform: scale(1.08);
    }
    67%,
    86% {
      transform: none;
      opacity: 1;
    }
    90%,
    100% {
      opacity: 0;
    }
  }
  @keyframes shackle {
    0%,
    58% {
      transform: translateY(-5px);
    }
    62%,
    100% {
      transform: none;
    }
  }
  @keyframes pulse {
    0%,
    63% {
      transform: scale(0.7);
      opacity: 0;
    }
    64% {
      opacity: 0.5;
    }
    82%,
    100% {
      transform: scale(1.6);
      opacity: 0;
    }
  }
</style>
