<script lang="ts">
  // The hero's film: the pains it answers and every feature, narrated, in the page's language. It plays muted, without
  // controls and in a loop, while it is on screen; its button turns the narration on, from the start. For people who ask
  // for less motion it stays still until they turn the sound on. Its files are kept in the releases' pool
  // (deploy/film.sh), which films.json names: AV1 and Opus for Chrome, Edge and Firefox, HEVC and AAC for Safari.
  import films from './films.json';
  import { reducedMotion } from './motion.ts';
  import { ORIGIN } from './site.ts';
  import { locale, t } from '../i18n/index.ts';

  const film = films[locale()];
  let video: HTMLVideoElement | undefined = $state();
  let sound = $state(false);

  $effect(() => {
    const node = video;
    if (!node) return;
    if (reducedMotion.current && !sound) {
      node.pause();
      return;
    }
    const observer = new IntersectionObserver(([entry]) => {
      if (entry.isIntersecting) node.play().catch(() => {});
      else node.pause();
    });
    observer.observe(node);
    return () => {
      observer.disconnect();
      node.pause();
    };
  });

  /** Turns the narration on, from the film's start, or off again. */
  function toggle(): void {
    const node = video;
    if (!node) return;
    sound = !sound;
    node.muted = !sound;
    if (sound) {
      node.currentTime = 0;
      node.play().catch(() => {
        sound = false;
        node.muted = true;
      });
    }
  }
</script>

<div class="tilt relative mt-10">
  <!-- Decorative: the page's own words say what it shows, and the film writes out every word it says. -->
  <video
    bind:this={video}
    class="block aspect-video w-full rounded-xl bg-wash shadow-[0_30px_80px_-40px_rgb(20_40_75/0.45)]"
    poster="{ORIGIN}{film.jpg}"
    muted
    loop
    playsinline
    disablepictureinpicture
    preload="metadata"
    aria-hidden="true"
  >
    <source src="{ORIGIN}{film.webm}" type={'video/webm; codecs="av01.0.08M.08, opus"'} />
    <source src="{ORIGIN}{film.mp4}" type={'video/mp4; codecs="hvc1.1.6.L120.90, mp4a.40.2"'} />
  </video>
  <button
    type="button"
    class="absolute bottom-3 left-3 inline-flex items-center gap-2 rounded-full bg-[#0a101c]/75 px-4 py-2 text-sm font-semibold text-white ring-1 ring-white/20 backdrop-blur hover:bg-[#0a101c]/90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent sm:bottom-5 sm:left-5 sm:px-5 sm:py-2.5 sm:text-base"
    onclick={toggle}
  >
    <svg class="size-5 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M4 9v6h4l5 4V5L8 9z" />
      {#if sound}
        <path d="M16 9a4 4 0 0 1 0 6M18.5 6.5a8 8 0 0 1 0 11" />
      {:else}
        <path d="M16 9.5l5 5M21 9.5l-5 5" />
      {/if}
    </svg>
    {sound ? t('page.film.soundOff') : t('page.film.soundOn')}
  </button>
</div>
