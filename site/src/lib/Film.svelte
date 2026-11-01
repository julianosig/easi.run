<script lang="ts">
  // The hero's film (src/video/, rendered by scripts/film.mjs): every feature in two minutes, in the page's language,
  // silent, without controls, in a loop. It plays while it is on screen; for people who ask for less motion, from the
  // start or later, it stays still. Its files are kept in the releases' pool (deploy/film.sh), which films.json names: AV1 for Chrome, Edge
  // and Firefox, HEVC for Safari.
  import films from './films.json';
  import { reducedMotion } from './motion.ts';
  import { ORIGIN } from './site.ts';
  import { locale } from '../i18n/index.ts';

  const film = films[locale()];
  let video: HTMLVideoElement | undefined = $state();

  $effect(() => {
    const node = video;
    if (!node) return;
    if (reducedMotion.current) {
      node.pause();
      return;
    }
    node.muted = true;
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
</script>

<div class="tilt mt-10">
  <!-- Decorative: the page's own words say what it shows. -->
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
    <source src="{ORIGIN}{film.webm}" type={'video/webm; codecs="av01.0.08M.08"'} />
    <source src="{ORIGIN}{film.mp4}" type={'video/mp4; codecs="hvc1.1.6.L120.90"'} />
  </video>
</div>
