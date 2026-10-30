<script lang="ts">
  // No clock running, drawn: a session's timeline runs on and on (its stripes flowing past toward the infinity sign),
  // the play mark steady. Decorative. The stripes loop every 3 s, moving only transforms; for less motion it stands
  // still.
</script>

<svg viewBox="-9 0 320 128" class="h-32 w-full" aria-hidden="true">
  <defs>
    <clipPath id="duration-track"><rect x="70" y="50" width="160" height="28" rx="14" /></clipPath>
  </defs>
  <!-- The play mark. -->
  <circle cx="44" cy="64" r="20" class="fill-accent" />
  <path d="M39 55v18l14-9z" class="fill-white" />

  <!-- The timeline, flowing without an end. -->
  <rect x="70" y="50" width="160" height="28" rx="14" class="fill-accent-soft" />
  <g clip-path="url(#duration-track)">
    <g class="flow">
      {#each Array.from({ length: 14 }, (_, i) => i) as i (i)}
        <rect x={52 + i * 24} y="58" width="14" height="12" rx="6" class="fill-accent/35" />
      {/each}
    </g>
  </g>
  <path d="M236 64c0-7 5-11 10-11 9 0 13 22 22 22 5 0 10-4 10-11s-5-11-10-11c-9 0-13 22-22 22-5 0-10-4-10-11z" class="fill-none stroke-ink" stroke-width="3" stroke-linejoin="round" />
</svg>

<style>
  .flow {
    animation: flow 3s linear infinite;
  }

  @keyframes flow {
    from {
      transform: none;
    }
    to {
      transform: translateX(24px);
    }
  }
</style>
