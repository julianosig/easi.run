// A card's drawing loops only while it is on screen: off it, app.css pauses its animations (data-off), so a page of
// many drawings costs no more than the few in view. With no script, or before it runs, every drawing plays.
import type { Attachment } from 'svelte/attachments';

export const playing: Attachment<HTMLElement> = (node) => {
  const observer = new IntersectionObserver(([entry]) => node.toggleAttribute('data-off', !entry.isIntersecting));
  observer.observe(node);
  return () => observer.disconnect();
};
