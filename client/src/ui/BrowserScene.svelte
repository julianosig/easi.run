<script lang="ts">
  // The landing's browser, drawn: a sign-in page where your agent (the blue cursor) types the email, then you take over
  // (the coral one) to type the password while the agent waits, as easi.run lets you. Decorative. It plays on a loop;
  // for people who ask for less motion it stands still, both cursors on the page. Sized in em, so the class's font size
  // scales it; it is wider and taller than its box, which crops it on the right and at the bottom.
  import { t } from '../i18n/index.svelte.ts';

  let { class: className = '' }: { class?: string } = $props();
  const ARROW = 'M12 6 56 30 35 36 25 58Z';
</script>

<div class="scene relative overflow-hidden text-[10px] sm:text-[14px] lg:text-[clamp(13px,1.05vw,16px)] {className}" aria-hidden="true">
  <div class="frame absolute left-[2px] top-[2px] h-[40em] w-[54em] overflow-hidden rounded-tl-[1.1em] bg-paper shadow-[0_30px_80px_-30px_rgb(20_40_75/0.45),0_0_0_1.5px_var(--color-accent)]">
    <!-- The tab strip and the toolbar, as Chrome draws them. -->
    <div class="flex h-[2.6em] items-end gap-[0.6em] bg-[#dce6ff] pl-[0.8em]">
      <span class="mb-[0.85em] flex gap-[0.4em]"
        ><i class="size-[0.7em] rounded-full bg-white/80"></i><i class="size-[0.7em] rounded-full bg-white/80"></i><i class="size-[0.7em] rounded-full bg-white/80"></i></span
      >
      <span class="flex h-[2em] w-[15em] items-center gap-[0.5em] rounded-t-[0.6em] bg-paper px-[0.8em] text-[0.85em] font-semibold text-ink/80">
        <svg viewBox="10 4 48 56" class="h-[1.1em] w-auto"><path d={ARROW} class="fill-accent" /></svg>{t('demo.scene.tab', { site: 'example.com' })}
      </span>
    </div>
    <div class="flex h-[2.8em] items-center gap-[0.9em] border-b border-line px-[1em] text-muted">
      <svg viewBox="0 0 24 24" class="size-[1.1em]" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6l-6 6 6 6" /></svg>
      <svg viewBox="0 0 24 24" class="size-[1.1em] opacity-50" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6" /></svg>
      <span class="flex h-[1.9em] w-[24em] items-center gap-[0.5em] rounded-full bg-wash px-[0.9em] text-[0.85em] text-ink/70">
        <svg viewBox="0 0 24 24" class="size-[1.05em]" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 11h12v9H6zM9 11V8a3 3 0 0 1 6 0v3" /></svg>example.com/sign-in
      </span>
      <span class="relative ml-[0.4em] h-[1.9em] w-[11em] text-[0.8em] font-semibold">
        <span class="chip-agent absolute inset-0 flex items-center gap-[0.5em] rounded-full bg-accent-soft px-[0.9em] text-accent"
          ><i class="size-[0.6em] rounded-full bg-accent"></i>{t('demo.scene.chip.agent')}</span
        >
        <span class="chip-you absolute inset-0 flex items-center gap-[0.5em] rounded-full bg-[#ffe9e3] px-[0.9em] text-[#c2401f]"
          ><i class="size-[0.6em] rounded-full bg-you"></i>{t('demo.scene.chip.you')}</span
        >
      </span>
    </div>

    <!-- The page. -->
    <div class="relative h-full bg-wash">
      <div class="absolute left-[6em] top-[2.2em] w-[22em] rounded-[1em] bg-paper p-[2em] shadow-[0_12px_32px_-18px_rgb(20_40_75/0.35)] ring-1 ring-line">
        <div class="flex h-[2.2em] items-center"><span class="text-[1.4em] font-bold tracking-[-0.02em] text-ink">{t('demo.scene.heading')}</span></div>
        <div class="mt-[0.5em] h-[0.6em] w-[12em] rounded-full bg-line"></div>
        <div class="mt-[1.2em] flex h-[1.2em] items-center"><span class="text-[0.85em] font-semibold text-ink/80">{t('demo.scene.email')}</span></div>
        <div class="mt-[0.4em] flex h-[2.6em] items-center overflow-hidden rounded-[0.6em] border border-line px-[0.8em] text-ink">
          <span class="email whitespace-nowrap">agent.mail@example.com</span>
        </div>
        <div class="mt-[0.9em] flex h-[1.2em] items-center"><span class="text-[0.85em] font-semibold text-ink/80">{t('demo.scene.password')}</span></div>
        <div class="field-you mt-[0.4em] flex h-[2.6em] items-center rounded-[0.6em] border border-line px-[0.8em] text-ink">
          <span class="secret whitespace-nowrap tracking-[0.2em]">••••••••••••</span>
        </div>
        <div class="press mt-[1.4em] grid h-[2.6em] place-items-center rounded-[0.6em] bg-accent font-semibold text-white">{t('demo.scene.submit')}</div>

        <!-- The cursors: their tip is the element's top left corner. -->
        <span class="cursor agent absolute left-0 top-0 flex items-start">
          <svg viewBox="10 4 48 56" class="h-[1.9em] w-auto drop-shadow-[0_2px_3px_rgb(20_40_75/0.3)]"><path d={ARROW} class="fill-accent stroke-white" stroke-width="4" stroke-linejoin="round" /></svg>
          <span class="ml-[0.1em] mt-[1.5em] rounded-full bg-accent px-[0.6em] py-[0.15em] text-[0.8em] font-semibold text-white">{t('demo.scene.cursor.agent')}</span>
        </span>
        <span class="cursor you absolute left-0 top-0 flex items-start">
          <svg viewBox="10 4 48 56" class="h-[1.9em] w-auto drop-shadow-[0_2px_3px_rgb(20_40_75/0.3)]"><path d={ARROW} class="fill-you stroke-white" stroke-width="4" stroke-linejoin="round" /></svg>
          <span class="ml-[0.1em] mt-[1.5em] rounded-full bg-you px-[0.6em] py-[0.15em] text-[0.8em] font-semibold text-white">{t('demo.scene.cursor.you')}</span>
        </span>
      </div>
    </div>
  </div>
</div>

<style>
  /* Turned a little away, as the landing's frame starts before it straightens. */
  @media (min-width: 64rem) {
    .frame {
      transform-origin: 0 50%;
      transform: perspective(1600px) rotateY(-9deg) rotateX(3deg);
    }
  }
  /* Standing still: the agent waits beside the form, you are at the password, typed. */
  .agent {
    translate: 24em 6em;
    opacity: 0.6;
  }
  .you {
    translate: 15em 14.5em;
  }
  .chip-agent {
    opacity: 0;
  }

  /* One loop of 12 s: the agent comes in and types the email (0–30 %), steps aside and waits; you come in, type the
     password and press Sign in (38–75 %), then leave, and the agent carries on. */
  @media (prefers-reduced-motion: no-preference) {
    .scene * {
      animation-duration: 12s;
      animation-iteration-count: infinite;
      animation-timing-function: cubic-bezier(0.65, 0, 0.35, 1);
    }
    .agent {
      animation-name: agent;
    }
    .you {
      animation-name: you;
    }
    .email {
      animation-name: email;
    }
    .secret {
      animation-name: secret;
    }
    .chip-agent {
      animation-name: chip-agent;
    }
    .chip-you {
      animation-name: chip-you;
    }
    .field-you {
      animation-name: field-you;
    }
    .press {
      animation-name: press;
    }
  }

  @keyframes agent {
    0% {
      translate: 30em -1em;
      opacity: 1;
    }
    9%,
    30% {
      translate: 10.5em 9.4em;
      opacity: 1;
    }
    36%,
    86% {
      translate: 24em 6em;
      opacity: 0.6;
    }
    100% {
      translate: 30em -1em;
      opacity: 1;
    }
  }
  @keyframes you {
    0%,
    34% {
      translate: 14em 30em;
      opacity: 0;
    }
    44%,
    60% {
      translate: 15em 14.5em;
      opacity: 1;
    }
    66%,
    76% {
      translate: 11em 18.5em;
      opacity: 1;
    }
    86%,
    100% {
      translate: 14em 30em;
      opacity: 0;
    }
  }
  @keyframes email {
    0%,
    10% {
      clip-path: inset(0 100% 0 0);
      animation-timing-function: steps(18, end);
    }
    28%,
    100% {
      clip-path: inset(0 0 0 0);
    }
  }
  @keyframes secret {
    0%,
    45% {
      clip-path: inset(0 100% 0 0);
      animation-timing-function: steps(12, end);
    }
    60%,
    92% {
      clip-path: inset(0 0 0 0);
    }
    100% {
      clip-path: inset(0 100% 0 0);
    }
  }
  @keyframes chip-agent {
    0%,
    34%,
    88%,
    100% {
      opacity: 1;
    }
    38%,
    84% {
      opacity: 0;
    }
  }
  @keyframes chip-you {
    0%,
    34%,
    88%,
    100% {
      opacity: 0;
    }
    38%,
    84% {
      opacity: 1;
    }
  }
  @keyframes field-you {
    0%,
    40%,
    64%,
    100% {
      border-color: var(--color-line);
      box-shadow: none;
    }
    44%,
    60% {
      border-color: var(--color-you);
      box-shadow: 0 0 0 0.25em rgb(240 100 69 / 0.18);
    }
  }
  @keyframes press {
    0%,
    70%,
    76%,
    100% {
      scale: 1;
      box-shadow: 0 0 0 0 rgb(240 100 69 / 0);
    }
    72% {
      scale: 0.97;
      box-shadow: 0 0 0 0.3em rgb(240 100 69 / 0.3);
    }
  }
</style>
