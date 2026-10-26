// The screen a session the admin page starts gets: a size real desktops have, so a site sees an ordinary screen.
// protocol/tests/screens.rs holds each to the output-size rule (SVT-AV1 crashes on some sizes).

/** 16:9, 16:10, 5:4 and 4:3 desktops' sizes. */
export const SCREENS: readonly (readonly [number, number])[] = [
  [1920, 1080],
  [1600, 900],
  [1536, 864],
  [1366, 768],
  [1280, 720],
  [1680, 1050],
  [1440, 900],
  [1280, 800],
  [1280, 1024],
  [1024, 768],
];

/** Of [`SCREENS`], those of the shape nearest a `screenWidth` x `screenHeight` device, and of them the one nearest a
 * `windowWidth` window. It is drawn scaled to the window, which may leave room around it. */
export function screenFor(screenWidth: number, screenHeight: number, windowWidth: number): { width: number; height: number } {
  const gap = ([w, h]: readonly [number, number]) => Math.abs(w / h - screenWidth / screenHeight);
  const nearest = Math.min(...SCREENS.map(gap));
  const alike = SCREENS.filter((s) => gap(s) - nearest < 0.01);
  const [width, height] = alike.reduce((best, s) => (Math.abs(s[0] - windowWidth) < Math.abs(best[0] - windowWidth) ? s : best));
  return { width, height };
}
