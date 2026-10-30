<script lang="ts">
  // The demo's front door: it starts a visit at once, at the first demo, without a password. While the browser opens
  // or the visit has to wait (every browser in use, or this address already has one), the page shows only the loading
  // shimmer; a visit that could not start says why and offers to try again.
  import { ICONS } from './icons.ts';
  import Button from './Button.svelte';
  import { api, AUTO_START, bodyOf, type DemoInfo, remember, remembered, setCsrf, statusOf, type Visit } from './api.ts';
  import { errorOf } from './errors.ts';
  import { i18n, t } from '../i18n/index.svelte.ts';
  import { demoOf } from './demos.ts';
  import { asked } from './embed.ts';
  import Loading from './Loading.svelte';

  let { demo, onVisit }: { demo: DemoInfo; onVisit: (visit: Visit) => void } = $props();

  const minutes = $derived(Math.round(demo.seconds / 60));
  // A page framing the demo may ask for the one to open first; the first demo opens otherwise.
  const picked = $derived(asked(demo.sites));
  const first = $derived(demoOf(picked ?? demo.sites[0] ?? ''));
  // What the page says, drawn in the current language (an error the server answered stays as it came).
  let status = $state<() => string>(() => t('demo.door.opening'));
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
      // The visit's browser speaks this page's language, which its demo pages then speak too.
      const body = { locale: i18n.locale, ...(picked ? { demo: picked } : {}), ...(auto ? { wait: true } : {}) };
      const login = await api<{ csrf: string; demo?: Visit | null; opening?: boolean }>('POST', '/api/login', body);
      setCsrf(login.csrf);
      if (login.demo) return onVisit(login.demo);
      // Its browser is opening, for the login this page now has: asked again, the server waits for it (as it does
      // for this page reloaded meanwhile).
      if (login.opening) return start();
      status = () => t('demo.door.failed');
      failed = true;
    } catch (e) {
      const code = statusOf(e);
      const body = bodyOf(e);
      if (code === 409 || code === 429) {
        const wait = Math.max(3, Math.min(30, Number(body.retry_after) || 5));
        const said =
          body.code === 'demo-yours'
            ? 'demo.door.wait.yours'
            : body.code === 'demo-often'
              ? 'demo.door.wait.often'
              : body.code === 'demo-full'
                ? 'demo.door.wait.full'
                : 'demo.door.wait.other';
        status = () => t(said);
        // A little jitter, so visitors who waited together do not all ask at once.
        retry = setTimeout(start, wait * 1000 + Math.random() * 1500);
      } else {
        status = errorOf(e);
        failed = true;
      }
    }
  }
  start();
</script>

{#if failed}
  <section class="mx-auto mt-20 max-w-xl text-center">
    <p class="inline-flex items-center gap-2 rounded-full bg-accent-soft px-3.5 py-1.5 text-sm font-semibold text-accent">
      <span class="size-1.5 rounded-full bg-you"></span>{t('demo.door.visit', { count: minutes, demo: t(first.title) })}
    </p>
    <p class="mt-6 text-lg font-semibold text-bad" role="status">{status()}</p>
    <Button variant="primary" icon={ICONS.retry} class="mt-6" onclick={start}>{t('demo.door.retry')}</Button>
    <p class="mx-auto mt-10 max-w-md text-sm text-muted">
      {t('demo.door.note', { count: minutes })}
    </p>
  </section>
{:else}
  <Loading said={status()} />
{/if}
