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
export const BUTTON_PINS = [0, 1, 4, 5, 11, 12, 13, 14];

/** GPIO pins assigned to each touch pad index in firmware. */
export const TOUCH_PINS = [6, 7, 8, 9, 10, 15, 16, 17];

/** GPIO (ADC) pins assigned to each pot index in firmware. */
export const POT_PINS = [26, 27];

/** GPIO (ADC) pin for the LDR in firmware. */
export const LDR_PIN = 28;

/** GPIO pins for the accelerometer I2C1 bus (SCL, SDA) in firmware. */
export const ACCEL_SCL_PIN = 3;
export const ACCEL_SDA_PIN = 2;

/** Format a GPIO pin number as a label, e.g. "GP2". */
export function pinLabel(n) {
  return n != null ? "GP" + n : "";
}

/** Wire up pin-label clicks within a container to open the pinout modal. */
export function wirePinClicks(container) {
  container.querySelectorAll(".pin-label.clickable").forEach(span => {
    span.addEventListener("click", () => {
      const gpio = parseInt(span.dataset.gpio, 10);
      const modal = document.querySelector("pinout-modal");
      if (modal && !isNaN(gpio)) modal.show(gpio);
    });
  });
}

/**
 * If `src` is a plain integer string, return the MIDI note name for it.
 * Otherwise return "".
 */
export function noteHintText(src) {
  const v = parseInt(src, 10);
  return (String(v) === (src || "").trim()) ? noteName(v) : "";
}

/** Toggle a fields container's display based on a checkbox state. */
export function toggleFieldsVisibility(root, checkboxId, fieldsId) {
  root.querySelector("#" + fieldsId).style.display =
    root.querySelector("#" + checkboxId).checked ? "" : "none";
}
