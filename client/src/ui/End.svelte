<script lang="ts">
  // The end of a demo visit: its time ran out (`over`), the visitor left (`left`), the connection dropped (`lost`: the
  // time keeps running on the server, which ends the browser on its own), or the visitor opened the demo in another
  // browser, which closed this one (`moved`). The page shows only a loading shimmer, as if the next browser were on its
  // way; the reason is still announced to screen readers, and picking a demo starts a new visit. When the time ran out
  // for someone who was using it, the page reloads itself a moment later and the next visit starts; a tab left alone
  // (or out of sight) waits for a touch first, so a forgotten page never keeps a browser busy. That start never closes
  // a visit the same address has open elsewhere: it waits for it (see Door).
  import { activeWithin } from './activity.ts';
  import { AUTO_START, remember } from './api.ts';
  import Loading from './Loading.svelte';

  let {
    why,
    seconds,
  }: {
    why: 'over' | 'lost' | 'left' | 'moved';
    /** A visit's length, as the server set it (ADMIN_DEMO_SECONDS). */
    seconds: number;
  } = $props();

  /** Long enough for the end to be announced before the page reloads. */
  const RELOAD_AFTER_MS = 2000;
  /** Added to a visit's length: a press within that long means the person was using the visit that ended. */
  const ACTIVE_MARGIN_MS = 60 * 1000;
  $effect(() => {
    if (why !== 'over') return;
    const restart = () => {
      // Without storage the next start is an ordinary one.
      remember(AUTO_START, '1', 'sessionStorage');
      location.reload();
    };
    if (activeWithin(seconds * 1000 + ACTIVE_MARGIN_MS) && !document.hidden) {
      const timer = setTimeout(restart, RELOAD_AFTER_MS);
      return () => clearTimeout(timer);
    }
    addEventListener('pointerdown', restart, { once: true });
    addEventListener('keydown', restart, { once: true });
    return () => {
      removeEventListener('pointerdown', restart);
      removeEventListener('keydown', restart);
    };
  });

  const said = $derived(
    why === 'over'
      ? 'Time’s up. That browser is being deleted, with everything in it.'
      : why === 'left'
        ? 'You left. That browser is being deleted, with everything in it.'
        : why === 'moved'
          ? 'Opened elsewhere. You opened the demo in another tab or browser, so this one closed.'
          : 'Connection lost. Your time keeps running on the server and ends on its own.',
  );
</script>

<Loading {said} />
