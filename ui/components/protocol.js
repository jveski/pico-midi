// COBS encode/decode and Postcard binary reader/writer.
// Matches the firmware's postcard + COBS framing exactly.

// ── COBS ──

export function cobsEncode(data) {
  const out = [];
  let codeIdx = 0, code = 1;
  out.push(0);
  for (let i = 0; i < data.length; i++) {
    if (data[i] === 0) {
      out[codeIdx] = code;
      codeIdx = out.length;
      out.push(0);
      code = 1;
    } else {
      out.push(data[i]);
      code++;
      if (code === 0xFF) {
        out[codeIdx] = code;
        codeIdx = out.length;
        out.push(0);
        code = 1;
      }
    }
  }
  out[codeIdx] = code;
  out.push(0x00);
  return new Uint8Array(out);
}

export function cobsDecode(buf) {
  const out = [];
  let i = 0;
  while (i < buf.length) {
    const code = buf[i++];
    if (code === 0) throw new Error("COBS: unexpected zero");
    for (let j = 1; j < code && i < buf.length; j++) out.push(buf[i++]);
    if (code < 0xFF && i < buf.length) out.push(0);
  }
  if (out.length > 0 && out[out.length - 1] === 0) out.pop();
  return new Uint8Array(out);
}

// ── Postcard Reader ──

export class PostcardReader {
  constructor(buf) { this.buf = buf; this.pos = 0; }
  u8() { return this.buf[this.pos++]; }
  bool() { return this.u8() !== 0; }
  varint() {
    let val = 0, shift = 0;
    while (true) {
      const b = this.u8();
      val |= (b & 0x7F) << shift;
      if ((b & 0x80) === 0) return val;
      shift += 7;
    }
  }
  str() {
    const len = this.varint();
    const bytes = this.buf.slice(this.pos, this.pos + len);
    this.pos += len;
    return new TextDecoder().decode(bytes);
  }
  remaining() { return this.buf.length - this.pos; }
}

// ── Postcard Writer ──

export class PostcardWriter {
  constructor() { this.buf = []; }
  u8(v) { this.buf.push(v & 0xFF); }
  bool(v) { this.u8(v ? 1 : 0); }
  varint(v) {
    v = v >>> 0;
    while (v >= 0x80) {
      this.buf.push((v & 0x7F) | 0x80);
      v >>>= 7;
    }
    this.buf.push(v & 0x7F);
  }
  finish() { return new Uint8Array(this.buf); }
}

// ── Protocol Constants ──

export const REQ_PING = 0;
export const REQ_VERSION = 1;
export const REQ_GET_CONFIG = 2;
export const REQ_PUT_CONFIG = 3;
export const REQ_SAVE = 4;
export const REQ_RESET = 5;
export const REQ_REBOOT = 6;

export const RESP_PONG = 0;
export const RESP_VERSION = 1;
export const RESP_CONFIG = 2;
export const RESP_OK = 3;
export const RESP_ERROR = 4;
export const RESP_MONITOR = 5;

export const MAX_BUTTONS = 8;
export const MAX_TOUCH_PADS = 8;
export const MAX_POTS = 4;

// ── Config Serialization ──

export function writeConfig(w, cfg) {
  w.u8(cfg.midi_channel);
  w.u8(cfg.buttons.length);
  for (let i = 0; i < MAX_BUTTONS; i++) {
    if (i < cfg.buttons.length) {
      w.u8(cfg.buttons[i].pin); w.u8(cfg.buttons[i].note); w.u8(cfg.buttons[i].velocity);
    } else { w.u8(0); w.u8(0); w.u8(0); }
  }
  w.u8(cfg.touch_pads.length);
  for (let i = 0; i < MAX_TOUCH_PADS; i++) {
    if (i < cfg.touch_pads.length) {
      w.u8(cfg.touch_pads[i].pin); w.u8(cfg.touch_pads[i].note); w.u8(cfg.touch_pads[i].velocity);
    } else { w.u8(0); w.u8(0); w.u8(0); }
  }
  w.u8(cfg.pots.length);
  for (let i = 0; i < MAX_POTS; i++) {
    if (i < cfg.pots.length) {
      w.u8(cfg.pots[i].pin); w.u8(cfg.pots[i].cc);
    } else { w.u8(0); w.u8(0); }
  }
  w.u8(cfg.ldr.pin); w.u8(cfg.ldr.cc);
  w.bool(cfg.ldr_enabled);
  w.bool(cfg.accel_enabled);
  w.u8(cfg.accel.sda); w.u8(cfg.accel.scl); w.u8(cfg.accel.int_pin);
  w.u8(cfg.accel.x_cc); w.u8(cfg.accel.y_cc);
  w.u8(cfg.accel.tap_note); w.u8(cfg.accel.tap_vel);
  w.u8(cfg.accel.dead_zone); w.u8(cfg.accel.smoothing);
}

export function readConfig(r) {
  const cfg = {
    midi_channel: 0, buttons: [], touch_pads: [], pots: [],
    ldr_enabled: false, ldr: { pin: 0, cc: 0 },
    accel_enabled: false,
    accel: { sda: 0, scl: 0, int_pin: 0, x_cc: 0, y_cc: 0, tap_note: 0, tap_vel: 1, dead_zone: 0, smoothing: 0 },
  };
  cfg.midi_channel = r.u8();
  const nb = r.u8();
  for (let i = 0; i < MAX_BUTTONS; i++) {
    const pin = r.u8(), note = r.u8(), velocity = r.u8();
    if (i < nb) cfg.buttons.push({ pin, note, velocity });
  }
  const nt = r.u8();
  for (let i = 0; i < MAX_TOUCH_PADS; i++) {
    const pin = r.u8(), note = r.u8(), velocity = r.u8();
    if (i < nt) cfg.touch_pads.push({ pin, note, velocity });
  }
  const np = r.u8();
  for (let i = 0; i < MAX_POTS; i++) {
    const pin = r.u8(), cc = r.u8();
    if (i < np) cfg.pots.push({ pin, cc });
  }
  cfg.ldr = { pin: r.u8(), cc: r.u8() };
  cfg.ldr_enabled = r.bool();
  cfg.accel_enabled = r.bool();
  cfg.accel = {
    sda: r.u8(), scl: r.u8(), int_pin: r.u8(),
    x_cc: r.u8(), y_cc: r.u8(),
    tap_note: r.u8(), tap_vel: r.u8(),
    dead_zone: r.u8(), smoothing: r.u8(),
  };
  return cfg;
}

export function readMonitorSnapshot(r) {
  const snap = { buttons: [], touch_pads: [], pots: [], ldr: 0, accel_x: 0, accel_y: 0, accel_tap: false };
  for (let i = 0; i < MAX_BUTTONS; i++) snap.buttons.push(r.bool());
  for (let i = 0; i < MAX_TOUCH_PADS; i++) snap.touch_pads.push(r.bool());
  for (let i = 0; i < MAX_POTS; i++) snap.pots.push(r.u8());
  snap.ldr = r.u8();
  snap.accel_x = r.u8();
  snap.accel_y = r.u8();
  snap.accel_tap = r.bool();
  return snap;
}

export function buildRequest(variantIndex, configObj) {
  const w = new PostcardWriter();
  w.varint(variantIndex);
  if (variantIndex === REQ_PUT_CONFIG && configObj) writeConfig(w, configObj);
  return cobsEncode(w.finish());
}

export function parseResponse(bytes) {
  const r = new PostcardReader(bytes);
  const variant = r.varint();
  switch (variant) {
    case RESP_PONG: return { type: "pong" };
    case RESP_VERSION: return { type: "version", value: r.str() };
    case RESP_CONFIG: return { type: "config", value: readConfig(r) };
    case RESP_OK: return { type: "ok" };
    case RESP_ERROR: return { type: "error", message: r.str() };
    case RESP_MONITOR: return { type: "monitor", value: readMonitorSnapshot(r) };
    default: return null;
  }
}
