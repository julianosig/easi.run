// Plays a 440 Hz tone for `seconds` (a page may start sound only after a gesture, so call it from one), then closes
// its audio context and calls `ended`.
export function play(seconds, ended) {
  const context = new AudioContext();
  const tone = context.createOscillator();
  const gain = context.createGain();
  tone.frequency.value = 440;
  gain.gain.value = 0.3;
  tone.connect(gain).connect(context.destination);
  tone.start();
  tone.stop(context.currentTime + seconds);
  tone.onended = () => { context.close(); ended(); };
}
