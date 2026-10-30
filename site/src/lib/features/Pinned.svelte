<script lang="ts">
  // Every image pinned, drawn: three images (containers, each with its digest beneath) and a pin that drops onto each
  // in turn. Decorative. It loops every 8 s, moving only transforms and opacity; for less motion it stands still with
  // every image pinned.
  const AT = [28, 124, 220];
</script>

<svg viewBox="0 0 320 128" class="h-32 w-full" aria-hidden="true">
  {#each AT as x, i (x)}
    <!-- A container image: a box, its lines, its digest. -->
    <path d="M{x} 54l36-14 36 14v40l-36 14-36-14z" class="fill-paper stroke-ink" stroke-width="2" stroke-linejoin="round" />
    <path d="M{x} 54l36 14 36-14M{x + 36} 68v40" class="fill-none stroke-ink" stroke-width="2" stroke-linejoin="round" />
    <rect x={x + 6} y="114" width="60" height="6" rx="3" class="fill-ink/20" />
    <!-- Its pin. -->
    <g class="pin n{i}">
      <circle cx={x + 36} cy="22" r="7" class="fill-you" />
      <path d="M{x + 36} 29v14" class="stroke-you" stroke-width="2.5" stroke-linecap="round" />
    </g>
  {/each}
</svg>

<style>
  .pin {
    animation: pin 8s cubic-bezier(0.4, 0, 0.6, 1.4) infinite;
  }
  .n1 {
    animation-delay: 0.7s;
  }
  .n2 {
    animation-delay: 1.4s;
  }

  @keyframes pin {
    0%,
    6% {
      transform: translateY(-26px);
      opacity: 0;
    }
    16%,
    84% {
      transform: none;
      opacity: 1;
    }
    94%,
    100% {
      transform: none;
      opacity: 0;
    }
  }
</style>
