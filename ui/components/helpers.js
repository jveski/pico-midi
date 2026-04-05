// Shared helper functions
export const NOTE_NAMES = ["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"];

export function noteName(n) {
  if (n < 0 || n > 127) return "";
  return NOTE_NAMES[n % 12] + (Math.floor(n / 12) - 1);
}

export function num(s, fallback) {
  const v = parseInt(s, 10);
  return isNaN(v) ? fallback : v;
}

export function clamp(v, min, max) {
  return Math.max(min, Math.min(max, v));
}

export function sleep(ms) {
  return new Promise(r => setTimeout(r, ms));
}
