// Shared helper functions
import { DIGITAL_PINS, ANALOG_PINS } from "./protocol.js";

// ── Base Element ──

/**
 * Base class for Web Components with an idempotent init guard.
 * Subclasses override `init()` instead of `connectedCallback()`.
 */
export class BaseElement extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.init();
  }

  /** Override in subclasses for one-time setup. */
  init() {}

  /** Dispatch a bubbling CustomEvent. */
  emit(name, detail) {
    this.dispatchEvent(new CustomEvent(name, { bubbles: true, detail }));
  }
}

/**
 * Define a boolean property backed by a CSS class on the element.
 * Usage: classProperty(MyElement, "visible", "visible");
 */
export function classProperty(Ctor, propName, className) {
  Object.defineProperty(Ctor.prototype, propName, {
    set(v) { this.classList.toggle(className, !!v); },
    get() { return this.classList.contains(className); },
  });
}

// ── Utilities ──

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

/**
 * Parse a string as a plain integer (surrounding whitespace is trimmed).
 * Returns the number, or null if it's not a plain integer.
 */
export function parseStaticInt(src) {
  const v = parseInt(src, 10);
  return (String(v) === (src || "").trim()) ? v : null;
}

/**
 * If `src` is a plain integer string, return the MIDI note name for it.
 * Otherwise return "".
 */
export function noteHintText(src) {
  const v = parseStaticInt(src);
  return v != null ? noteName(v) : "";
}

// ── Hardware pin pools (mirrors firmware config.rs) ──

/** GPIO pins for the accelerometer I2C1 bus (SCL, SDA) in firmware. */
export const ACCEL_SCL_PIN = 3;
export const ACCEL_SDA_PIN = 2;

/** Format a GPIO pin number as a label, e.g. "GP2". */
export function pinLabel(n) {
  return n != null ? "GP" + n : "";
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
 * Build <option> elements for a pin selector.
 * @param {number[]} pinList - Available pin numbers.
 * @param {number} currentPin - The pin currently assigned to this item.
 * @param {Set<number>} usedPins - Pins in use by other items (excludes currentPin).
 * @returns {string} HTML string of <option> elements.
 */
function pinOptions(pinList, currentPin, usedPins = new Set()) {
  return pinList.map(p => {
    const inUse = usedPins.has(p) && p !== currentPin;
    return `<option value="${p}" ${p === currentPin ? "selected" : ""} ${inUse ? "disabled" : ""}>GP${p}</option>`;
  }).join("");
}

export const digitalPinOptions = (currentPin, usedPins) => pinOptions(DIGITAL_PINS, currentPin, usedPins);
export const analogPinOptions  = (currentPin, usedPins) => pinOptions(ANALOG_PINS, currentPin, usedPins);

/**
 * Find the first available (unused) pin from a pin list.
 * Returns the pin number, or pinList[0] if all are taken.
 */
function nextAvailablePin(pinList, usedPins) {
  for (const p of pinList) {
    if (!usedPins.has(p)) return p;
  }
  return pinList[0];
}

export const nextAvailableDigitalPin = (usedPins) => nextAvailablePin(DIGITAL_PINS, usedPins);
export const nextAvailableAnalogPin  = (usedPins) => nextAvailablePin(ANALOG_PINS, usedPins);
