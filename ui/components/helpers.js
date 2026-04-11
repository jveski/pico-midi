import { DIGITAL_PINS, ANALOG_PINS } from "./protocol.js";

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

  init() {}

  emit(name, detail) {
    this.dispatchEvent(new CustomEvent(name, { bubbles: true, detail }));
  }
}

export function classProperty(Ctor, propName, className) {
  Object.defineProperty(Ctor.prototype, propName, {
    set(v) { this.classList.toggle(className, !!v); },
    get() { return this.classList.contains(className); },
  });
}

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

export function parseStaticInt(src) {
  const v = parseInt(src, 10);
  return (String(v) === (src || "").trim()) ? v : null;
}

export function noteHintText(src) {
  const v = parseStaticInt(src);
  return v != null ? noteName(v) : "";
}

export const ACCEL_SCL_PIN = 3;
export const ACCEL_SDA_PIN = 2;

export function pinLabel(n) {
  return n != null ? "GP" + n : "";
}

export function toggleFieldsVisibility(root, checkboxId, fieldsId) {
  root.querySelector("#" + fieldsId).style.display =
    root.querySelector("#" + checkboxId).checked ? "" : "none";
}

export function usedDigitalPins(cfg) {
  const used = new Set();
  for (const b of cfg.buttons) used.add(b.pin);
  for (const t of cfg.touch_pads) used.add(t.pin);
  return used;
}

export function usedAnalogPins(cfg) {
  const used = new Set();
  for (const p of cfg.pots) used.add(p.pin);
  if (cfg.ldr_enabled) used.add(cfg.ldr.pin);
  return used;
}

function pinOptions(pinList, currentPin, usedPins = new Set()) {
  return pinList.map(p => {
    const inUse = usedPins.has(p) && p !== currentPin;
    return `<option value="${p}" ${p === currentPin ? "selected" : ""} ${inUse ? "disabled" : ""}>GP${p}</option>`;
  }).join("");
}

export const digitalPinOptions = (currentPin, usedPins) => pinOptions(DIGITAL_PINS, currentPin, usedPins);
export const analogPinOptions  = (currentPin, usedPins) => pinOptions(ANALOG_PINS, currentPin, usedPins);

function nextAvailablePin(pinList, usedPins) {
  for (const p of pinList) {
    if (!usedPins.has(p)) return p;
  }
  return pinList[0];
}

export const nextAvailableDigitalPin = (usedPins) => nextAvailablePin(DIGITAL_PINS, usedPins);
export const nextAvailableAnalogPin  = (usedPins) => nextAvailablePin(ANALOG_PINS, usedPins);
