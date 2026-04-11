// Shared helper functions
import { DIGITAL_PINS, ANALOG_PINS } from "./protocol.js";

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

// ── Hardware pin pools (mirrors firmware config.rs) ──

/** GPIO pins for the accelerometer I2C1 bus (SCL, SDA) in firmware. */
export const ACCEL_SCL_PIN = 3;
export const ACCEL_SDA_PIN = 2;

/** Format a GPIO pin number as a label, e.g. "GP2". */
export function pinLabel(n) {
  return n != null ? "GP" + n : "";
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

// ── Pin constraint helpers ──

/**
 * Get the list of digital pin numbers currently in use across the config.
 * Includes button pins and touch pad pins.
 */
export function usedDigitalPins(cfg) {
  const used = new Set();
  for (const b of cfg.buttons) used.add(b.pin);
  for (const t of cfg.touch_pads) used.add(t.pin);
  return used;
}

/**
 * Get the list of analog pin numbers currently in use across the config.
 * Includes pot pins and (if enabled) the LDR pin.
 */
export function usedAnalogPins(cfg) {
  const used = new Set();
  for (const p of cfg.pots) used.add(p.pin);
  if (cfg.ldr_enabled) used.add(cfg.ldr.pin);
  return used;
}

/**
 * Build <option> elements for a digital pin selector.
 * @param {number} currentPin - The pin currently assigned to this item.
 * @param {Set<number>} usedPins - Pins in use by other items (excludes currentPin).
 * @returns {string} HTML string of <option> elements.
 */
export function digitalPinOptions(currentPin, usedPins) {
  return DIGITAL_PINS.map(p => {
    const inUse = usedPins.has(p) && p !== currentPin;
    return `<option value="${p}" ${p === currentPin ? "selected" : ""} ${inUse ? "disabled" : ""}>GP${p}</option>`;
  }).join("");
}

/**
 * Build <option> elements for an analog pin selector.
 * @param {number} currentPin - The pin currently assigned to this item.
 * @param {Set<number>} usedPins - Pins in use by other items (excludes currentPin).
 * @returns {string} HTML string of <option> elements.
 */
export function analogPinOptions(currentPin, usedPins) {
  return ANALOG_PINS.map(p => {
    const inUse = usedPins.has(p) && p !== currentPin;
    return `<option value="${p}" ${p === currentPin ? "selected" : ""} ${inUse ? "disabled" : ""}>GP${p}</option>`;
  }).join("");
}

/**
 * Find the first available (unused) digital pin.
 * Returns the pin number, or DIGITAL_PINS[0] if all are taken.
 */
export function nextAvailableDigitalPin(usedPins) {
  for (const p of DIGITAL_PINS) {
    if (!usedPins.has(p)) return p;
  }
  return DIGITAL_PINS[0];
}

/**
 * Find the first available (unused) analog pin.
 * Returns the pin number, or ANALOG_PINS[0] if all are taken.
 */
export function nextAvailableAnalogPin(usedPins) {
  for (const p of ANALOG_PINS) {
    if (!usedPins.has(p)) return p;
  }
  return ANALOG_PINS[0];
}
