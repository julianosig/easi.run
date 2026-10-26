<script lang="ts">
  // The hero's live demo: the demo itself in a frame, and above it a menu of the demos, the one on marked. The frame tells this page
  // its demos and the one on (`easi-run:demos`); a pick goes back to it (`easi-run:pick`). Each side checks the other's
  // origin, and what the frame says is checked before it is shown.
  import { DEMO } from './site.ts';
  import { focus } from './focus.ts';

  type Demo = { name: string; title: string; line: string; icon: string };
  /** The frame's origin, read in the browser (the page is also rendered at its build). */
  const origin = () => new URL(DEMO.url, location.href).origin;

  let frame: HTMLIFrameElement | undefined = $state();
  let demos: Demo[] = $state([]);
  let current = $state('');
  /** A visit is under way in the frame to switch; otherwise a pick reloads the frame at that demo. */
  let active = $state(false);
  let src = $state(DEMO.url);

  const text = (v: unknown, max: number): v is string => typeof v === 'string' && v.length <= max;
  // An icon is an SVG path in a 24-unit box: path commands and numbers only.
  const icon = (v: unknown): v is string => text(v, 300) && /^[MLHVCSQTAZmlhvcsqtaz0-9 .,-]*$/.test(v);

  $effect(() => {
    const listener = (e: MessageEvent) => {
      if (e.origin !== origin() || e.source !== frame?.contentWindow) return;
      const data = e.data as { type?: unknown; demos?: unknown; current?: unknown; active?: unknown } | null;
      if (data?.type !== 'easi-run:demos' || !Array.isArray(data.demos) || data.demos.length > 16 || !text(data.current, 32)) return;
      if (typeof data.active !== 'boolean') return;
      const list = data.demos as Record<string, unknown>[];
      if (!list.every((d) => text(d.name, 32) && text(d.title, 40) && text(d.line, 80) && icon(d.icon))) return;
      demos = list.map((d) => ({ name: d.name as string, title: d.title as string, line: d.line as string, icon: d.icon as string }));
      current = data.current;
      active = data.active;
    };
    window.addEventListener('message', listener);
    // The frame may have spoken before this page listened (the page comes rendered, the frame loading at once).
    frame?.contentWindow?.postMessage({ type: 'easi-run:ask' }, origin());
    return () => window.removeEventListener('message', listener);
  });

  function pick(name: string) {
    if (name === current && active) return;
    current = name;
    if (active) frame?.contentWindow?.postMessage({ type: 'easi-run:pick', demo: name }, origin());
    else src = `${DEMO.url}?demo=${encodeURIComponent(name)}&at=${Date.now()}`;
  }
</script>

<!-- The demos, one item each, the one on marked. -->
<nav class="mt-8 flex min-h-11 flex-wrap items-center justify-center gap-1.5" aria-label="Demos">
  {#each demos as d (d.name)}
    <button
      class="flex items-center gap-2 rounded-full border px-3.5 py-2 text-sm font-semibold transition {d.name === current
        ? 'border-accent/30 bg-accent-soft text-accent'
        : 'border-transparent text-muted hover:bg-wash hover:text-ink'} {focus}"
      type="button"
      aria-current={d.name === current ? 'true' : undefined}
      title={d.line}
      onclick={() => pick(d.name)}
    >
      <svg viewBox="0 0 24 24" class="size-4 shrink-0" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={d.icon} /></svg>
      {d.title}
    </button>
  {/each}
</nav>

<div class="tilt mt-10">
  <!-- The real thing: the public demo, live, in my viewer. -->
  <iframe
    bind:this={frame}
    {src}
    title="easi.run live demo: a real Chrome on my server, yours to use"
    class="block aspect-[16/10] w-full rounded-xl bg-[#0f1b33]"
    allow="autoplay; clipboard-read; clipboard-write; camera; microphone; fullscreen"
    loading="lazy"
  ></iframe>
</div>
