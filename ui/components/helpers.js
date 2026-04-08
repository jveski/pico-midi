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

// ── Hardware pin mappings (mirrors firmware main.rs) ──

/** GPIO pins assigned to each button index in firmware. */
export const BUTTON_PINS = [2, 3, 4, 5, 11, 12, 13, 14];

/** GPIO pins assigned to each touch pad index in firmware. */
export const TOUCH_PINS = [6, 7, 8, 9, 10, 15, 16, 17];

/** GPIO (ADC) pins assigned to each pot index in firmware. */
export const POT_PINS = [26, 27];

/** GPIO pin pairs [A, B] assigned to each encoder index in firmware. */
export const ENCODER_PINS = [[18, 19], [20, 21]];

/** GPIO (ADC) pin for the LDR in firmware. */
export const LDR_PIN = 28;

/** Format a GPIO pin number (or pair) as a label, e.g. "GP2" or "GP18/19". */
export function pinLabel(n) {
  if (Array.isArray(n)) return "GP" + n[0] + "/" + n[1];
  return n != null ? "GP" + n : "";
}
