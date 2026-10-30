<script lang="ts">
  // Speaking your language, drawn: a speech bubble whose word turns from one script to the next (Latin, Devanagari,
  // Han, Hangul, Cyrillic, Kana), and the globe beside it. Decorative. It loops every 9 s, moving only transforms and
  // opacity; for less motion it stands still on the first.
  const WORDS = ['Aa', 'अ', '文', '한', 'Я', 'あ'];
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  <!-- The globe. -->
  <g transform="translate(96 64)" class="fill-none stroke-accent" stroke-width="2.5">
    <circle r="30" />
    <path d="M-30 0h60M0 -30c9 8 13 18 13 30s-4 22-13 30c-9-8-13-18-13-30s4-22 13-30z" />
  </g>
  <!-- The bubble, and its word. -->
  <path d="M156 28h96a12 12 0 0 1 12 12v40a12 12 0 0 1-12 12h-68l-16 14v-14h-12a12 12 0 0 1-12-12V40a12 12 0 0 1 12-12z" class="fill-accent-soft" />
  {#each WORDS as word, i (word)}
    <text x="204" y="72" text-anchor="middle" class="word w{i} fill-ink font-sans text-[32px] font-bold">{word}</text>
  {/each}
</svg>

<style>
  .word {
    opacity: 0;
    animation: word 9s ease-in-out infinite;
  }
  /* The still frame: the first word alone. */
  .w0 {
    opacity: 1;
  }
  .w1 {
    animation-delay: 1.5s;
  }
  .w2 {
    animation-delay: 3s;
  }
  .w3 {
    animation-delay: 4.5s;
  }
  .w4 {
    animation-delay: 6s;
  }
  .w5 {
    animation-delay: 7.5s;
  }

  @keyframes word {
    0% {
      transform: translateY(6px);
      opacity: 0;
    }
    4%,
    14% {
      transform: none;
      opacity: 1;
    }
    18%,
    100% {
      transform: translateY(-6px);
      opacity: 0;
    }
  }
</style>
