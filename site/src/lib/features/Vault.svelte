<script lang="ts">
  // Your card, drawn: a vault. Its wheel turns to unlock it, its heavy door swings open on its hinges (in perspective,
  // as a real door turns), a logged-in session (a check and a key) drops inside, the door swings shut and the wheel
  // turns back to lock it, with a soft pulse. Decorative. It loops every 8 s, moving only transforms and opacity; for
  // people who ask for less motion it stands still on the locked vault (app.css stops every animation, and what is
  // left is the frame each element is drawn at).
</script>

<!-- A box of the drawing's own size, centred in the card: the door is laid over the body exactly. -->
<div class="vault relative mx-auto h-32 w-40" aria-hidden="true">
  <svg viewBox="0 0 160 128" class="absolute inset-0 h-full w-full overflow-visible">
    <!-- The body on two feet, and its dark inside, seen when the door is open. -->
    <rect x="37" y="118" width="14" height="6" rx="2" class="fill-ink" />
    <rect x="109" y="118" width="14" height="6" rx="2" class="fill-ink" />
    <rect x="23" y="8" width="114" height="112" rx="14" class="fill-paper stroke-ink" stroke-width="2.5" />
    <rect x="33" y="18" width="94" height="92" rx="8" class="fill-ink/80" />

    <!-- The session: logged in (the check), with its key; drawn where it rests inside, dropped in from above. -->
    <g class="session">
      <rect x="40" y="40" width="80" height="48" rx="8" class="fill-paper stroke-line" stroke-width="1.5" />
      <circle cx="60" cy="64" r="9" class="fill-accent" />
      <path d="m56 64 3 3 5-6" class="fill-none stroke-white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
      <circle cx="84" cy="64" r="5" class="fill-none stroke-ink" stroke-width="2.4" />
      <path d="M89 64h18m-4 0v5m4-5v4" class="fill-none stroke-ink" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" />
    </g>
    <circle cx="80" cy="64" r="26" class="pulse fill-none stroke-accent" stroke-width="2" />
  </svg>

  <!-- The door, hinged on the left: it turns about that edge in perspective, its face shortening as a door's does. -->
  <div class="door absolute left-[33px] top-[18px] h-[92px] w-[94px]">
    <svg viewBox="0 0 94 92" class="h-full w-full overflow-visible">
      <rect x="1" y="1" width="92" height="90" rx="8" class="fill-paper stroke-ink" stroke-width="2" />
      <rect x="7" y="7" width="80" height="78" rx="5" class="fill-none stroke-ink/15" stroke-width="1.5" />
      <!-- Hinges on the left edge. -->
      <rect x="-3" y="16" width="6" height="14" rx="2" class="fill-ink" />
      <rect x="-3" y="62" width="6" height="14" rx="2" class="fill-ink" />
      <!-- The wheel, which turns to lock and unlock. -->
      <g class="wheel">
        <circle cx="47" cy="46" r="15" class="fill-none stroke-ink" stroke-width="3" />
        <path d="M47 31v30M32 46h30M36.4 35.4l21.2 21.2M57.6 35.4 36.4 56.6" class="stroke-ink" stroke-width="2.4" stroke-linecap="round" />
        <circle cx="47" cy="46" r="4.5" class="fill-accent" />
      </g>
    </svg>
  </div>
</div>

<style>
  /* The still frame: the door shut, the session inside, the wheel at rest. */
  .vault {
    perspective: 360px;
  }
  .door {
    transform-origin: 0 50%;
    transform-style: preserve-3d;
    animation: door 8s cubic-bezier(0.55, 0, 0.3, 1) infinite;
  }
  .wheel {
    transform-box: fill-box;
    transform-origin: center;
    animation: wheel 8s ease-in-out infinite;
  }
  .session {
    opacity: 0;
    animation: session 8s ease-in-out infinite;
  }
  .pulse {
    transform-box: fill-box;
    transform-origin: center;
    opacity: 0;
    animation: pulse 8s ease-out infinite;
  }

  /* Unlock (the wheel turns), open (the door swings out on its hinges), shut, lock (the wheel turns back). */
  @keyframes wheel {
    0%,
    4% {
      transform: none;
    }
    16%,
    70% {
      transform: rotate(-270deg);
    }
    82%,
    100% {
      transform: none;
    }
  }
  @keyframes door {
    0%,
    16% {
      transform: none;
    }
    32%,
    54% {
      transform: rotateY(-112deg);
    }
    68%,
    100% {
      transform: none;
    }
  }
  @keyframes session {
    0%,
    30% {
      transform: translateY(-62px);
      opacity: 0;
    }
    34% {
      transform: translateY(-62px);
      opacity: 1;
    }
    48%,
    92% {
      transform: none;
      opacity: 1;
    }
    96%,
    100% {
      transform: none;
      opacity: 0;
    }
  }
  @keyframes pulse {
    0%,
    82% {
      transform: scale(0.8);
      opacity: 0;
    }
    84% {
      opacity: 0.55;
    }
    98%,
    100% {
      transform: scale(1.7);
      opacity: 0;
    }
  }
</style>
