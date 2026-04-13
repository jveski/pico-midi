// COBS encode/decode and Postcard binary reader/writer.
// Matches the firmware's postcard + COBS framing exactly.

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

export class PostcardReader {
  constructor(buf) { this.buf = buf; this.pos = 0; }
  u8() { return this.buf[this.pos++]; }
  i8() { const v = this.u8(); return v > 127 ? v - 256 : v; }
  bool() { return this.u8() !== 0; }
  u16() { return this.varint(); }
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

export class PostcardWriter {
  constructor() { this.buf = []; }
  u8(v) { this.buf.push(v & 0xFF); }
  i8(v) { this.u8(v < 0 ? v + 256 : v); }
  bool(v) { this.u8(v ? 1 : 0); }
  u16(v) { this.varint(v); }
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

export const REQ_PING = 0;
export const REQ_VERSION = 1;
export const REQ_GET_CONFIG = 2;
export const REQ_PUT_CONFIG = 3;
export const REQ_SAVE = 4;
export const REQ_RESET = 5;

export const RESP_PONG = 0;
export const RESP_VERSION = 1;
export const RESP_CONFIG = 2;
export const RESP_OK = 3;
export const RESP_ERROR = 4;
export const RESP_MONITOR = 5;

export const MAX_DIGITAL_INPUTS = 21;
export const MAX_ANALOG_INPUTS = 3;
export const MAX_EXPR = 16;

export const DIGITAL_PINS = [0, 1, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22];

export const ANALOG_PINS = [26, 27, 28];

function writeExpr(w, expr) {
  const len = expr ? expr.length : 0;
  w.u8(len);
  for (let i = 0; i < MAX_EXPR; i++) {
    w.u8(i < len ? expr[i] : 0);
  }
}

function readExpr(r) {
  const len = r.u8();
  const code = [];
  for (let i = 0; i < MAX_EXPR; i++) {
    code.push(r.u8());
  }
  return { len, code: code.slice(0, len) };
}

// Config serialization — must match firmware Config struct field order exactly:
//   midi_channel, num_buttons, buttons[MAX_DIGITAL_INPUTS],
//   num_touch_pads, touch_pads[MAX_DIGITAL_INPUTS],
//   num_pots, pots[MAX_ANALOG_INPUTS],
//   ldr_enabled, ldr, accel, synth

function writeInputBlock(w, items, maxLen, defaultItem, writeExtra) {
  w.u8(items.length);
  for (let i = 0; i < maxLen; i++) {
    const item = i < items.length ? items[i] : defaultItem;
    w.u8(item.pin); w.u8(item.note); w.u8(item.velocity);
    if (writeExtra) writeExtra(w, item);
    writeExpr(w, item.note_expr);
    writeExpr(w, item.velocity_expr);
  }
}

function readInputBlock(r, maxLen, readExtra) {
  const count = r.u8();
  const all = [];
  for (let i = 0; i < maxLen; i++) {
    const pin = r.u8(), note = r.u8(), velocity = r.u8();
    const extra = readExtra ? readExtra(r) : {};
    const note_expr = readExpr(r);
    const velocity_expr = readExpr(r);
    all.push({ pin, note, velocity, ...extra, note_expr: note_expr.code, velocity_expr: velocity_expr.code });
  }
  return all.slice(0, count);
}

function readSlicedArray(r, maxLen, readFn) {
  const count = r.u8();
  const all = [];
  for (let i = 0; i < maxLen; i++) all.push(readFn(r));
  return all.slice(0, count);
}

export function writeConfig(w, cfg) {
  w.u8(cfg.midi_channel);

  writeInputBlock(w, cfg.buttons, MAX_DIGITAL_INPUTS,
    { pin: 0, note: 60, velocity: 100, note_expr: [], velocity_expr: [] });
  writeInputBlock(w, cfg.touch_pads, MAX_DIGITAL_INPUTS,
    { pin: 0, note: 48, velocity: 100, threshold_pct: 33, note_expr: [], velocity_expr: [] },
    (w, t) => w.u8(t.threshold_pct));

  const numPots = cfg.pots.length;
  w.u8(numPots);
  for (let i = 0; i < MAX_ANALOG_INPUTS; i++) {
    const p = i < numPots ? cfg.pots[i] : { pin: 0, cc: 0 };
    w.u8(p.pin); w.u8(p.cc);
  }

  w.bool(cfg.ldr_enabled);
  w.u8(cfg.ldr.pin); w.u8(cfg.ldr.cc);

  w.bool(cfg.accel.enabled);
  w.u8(cfg.accel.x_cc); w.u8(cfg.accel.y_cc);
  w.u8(cfg.accel.tap_note); w.u8(cfg.accel.tap_velocity);
  w.u8(cfg.accel.dead_zone_tenths); w.u8(cfg.accel.smoothing_pct);

  w.bool(cfg.synth.enabled);
  w.u8(cfg.synth.audio_pin);
  w.u8(cfg.synth.osc1_waveform); w.u8(cfg.synth.osc2_waveform);
  w.i8(cfg.synth.osc2_detune_cents); w.i8(cfg.synth.osc2_semitone);
  w.u8(cfg.synth.osc_mix);
  w.u8(cfg.synth.filter_cutoff); w.u8(cfg.synth.filter_resonance);
  w.u8(cfg.synth.filter_env_amount);
  w.u16(cfg.synth.amp_attack_ms); w.u16(cfg.synth.amp_decay_ms);
  w.u8(cfg.synth.amp_sustain_pct);
  w.u16(cfg.synth.amp_release_ms);
  w.u16(cfg.synth.filter_attack_ms); w.u16(cfg.synth.filter_decay_ms);
  w.u8(cfg.synth.filter_sustain_pct);
  w.u16(cfg.synth.filter_release_ms);
  w.u8(cfg.synth.master_volume);
}

export function readConfig(r) {
  const cfg = {
    midi_channel: r.u8(),
    buttons: readInputBlock(r, MAX_DIGITAL_INPUTS),
    touch_pads: readInputBlock(r, MAX_DIGITAL_INPUTS, r => ({ threshold_pct: r.u8() })),
    pots: readSlicedArray(r, MAX_ANALOG_INPUTS, r => ({ pin: r.u8(), cc: r.u8() })),
    ldr_enabled: r.bool(),
    ldr: { pin: r.u8(), cc: r.u8() },
    accel: {
      enabled: r.bool(),
      x_cc: r.u8(), y_cc: r.u8(),
      tap_note: r.u8(), tap_velocity: r.u8(),
      dead_zone_tenths: r.u8(), smoothing_pct: r.u8(),
    },
    synth: {
      enabled: r.bool(),
      audio_pin: r.u8(),
      osc1_waveform: r.u8(), osc2_waveform: r.u8(),
      osc2_detune_cents: r.i8(), osc2_semitone: r.i8(),
      osc_mix: r.u8(),
      filter_cutoff: r.u8(), filter_resonance: r.u8(),
      filter_env_amount: r.u8(),
      amp_attack_ms: r.u16(), amp_decay_ms: r.u16(),
      amp_sustain_pct: r.u8(),
      amp_release_ms: r.u16(),
      filter_attack_ms: r.u16(), filter_decay_ms: r.u16(),
      filter_sustain_pct: r.u8(),
      filter_release_ms: r.u16(),
      master_volume: r.u8(),
    },
  };
  return cfg;
}

export function readMonitorSnapshot(r) {
  const snap = {
    buttons: readSlicedArray(r, MAX_DIGITAL_INPUTS, r => r.bool()),
    touch_pads: readSlicedArray(r, MAX_DIGITAL_INPUTS, r => r.bool()),
    pots: readSlicedArray(r, MAX_ANALOG_INPUTS, r => r.u8()),
    ldr: r.u8(),
    accel_x: r.u8(),
    accel_y: r.u8(),
    accel_tap: r.bool(),
  };
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
