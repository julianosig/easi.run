<script lang="ts">
  // The demo's front door: it starts a visit at once, at the first demo, without a password. While the browser opens
  // or the visit has to wait (every browser in use, or this address already has one), the page shows only the loading
  // shimmer; a visit that could not start says why and offers to try again.
  import { api, AUTO_START, bodyOf, type DemoInfo, messageOf, remember, remembered, setCsrf, statusOf, type Visit } from './api.ts';
  import { demoOf } from './demos.ts';
  import { asked } from './embed.ts';
  import Loading from './Loading.svelte';
  import { primary } from './ui.ts';

  let { demo, onVisit }: { demo: DemoInfo; onVisit: (visit: Visit) => void } = $props();

  const minutes = $derived(Math.round(demo.seconds / 60));
  const first = $derived(demoOf(demo.sites[0] ?? ''));
  let status = $state('Opening your browser…');
  let failed = $state(false);
  let retry: ReturnType<typeof setTimeout> | undefined;
  $effect(() => () => clearTimeout(retry));
  // A start the page makes on its own, after the last visit's time ran out: it waits for a visit this address has open
  // elsewhere instead of closing it.
  const auto = remembered(AUTO_START, 'sessionStorage') === '1';
  remember(AUTO_START, null, 'sessionStorage');

  async function start() {
    clearTimeout(retry);
    failed = false;
    try {
      // A page framing the demo may ask for the one to open first.
      const first = asked(demo.sites);
      const body = { ...(first ? { demo: first } : {}), ...(auto ? { wait: true } : {}) };
      const login = await api<{ csrf: string; demo?: Visit | null }>('POST', '/api/login', body);
      setCsrf(login.csrf);
      if (login.demo) return onVisit(login.demo);
      status = 'The demo did not start.';
      failed = true;
    } catch (e) {
      const code = statusOf(e);
      const body = bodyOf(e);
      if (code === 409 || code === 429) {
        const wait = Math.max(3, Math.min(30, Number(body.retry_after) || 5));
        status =
          body.reason === 'yours'
            ? 'Closing your previous demo…'
            : body.reason === 'often'
              ? 'You started many demos in a row. The next one starts in a moment.'
              : body.reason === 'full'
                ? 'Every demo browser is in use. Yours starts on its own as soon as one is free.'
                : 'The demo is opening…';
        // A little jitter, so visitors who waited together do not all ask at once.
        retry = setTimeout(start, wait * 1000 + Math.random() * 1500);
      } else {
        status = messageOf(e);
        failed = true;
      }
    }
  }
  start();
</script>

{#if failed}
  <section class="mx-auto mt-20 max-w-xl text-center">
    <p class="inline-flex items-center gap-2 rounded-full bg-accent-soft px-3.5 py-1.5 text-sm font-semibold text-accent">
      <span class="size-1.5 rounded-full bg-you"></span>{minutes} minutes · {first.title}
    </p>
    <p class="mt-6 text-lg font-semibold text-bad" role="status">{status}</p>
    <button class="{primary} mt-6" type="button" onclick={start}>Try again</button>
    <p class="mx-auto mt-10 max-w-md text-sm text-muted">
      A real Chrome on my server, yours for {minutes} minutes. Use sample data only, never real accounts or files.
    </p>
  </section>
{:else}
  <Loading said={status} />
{/if}
