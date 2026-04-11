import {
  cobsDecode, buildRequest, parseResponse, readMonitorSnapshot,
  PostcardReader,
  REQ_VERSION, REQ_GET_CONFIG, REQ_PUT_CONFIG, REQ_SAVE, REQ_RESET,
  RESP_MONITOR, MAX_DIGITAL_INPUTS, MAX_ANALOG_INPUTS,
  DIGITAL_PINS, ANALOG_PINS,
} from "./protocol.js";
import { sleep, num, clamp, parseStaticInt, usedDigitalPins, usedAnalogPins, nextAvailableDigitalPin, nextAvailableAnalogPin } from "./helpers.js";
import { compileExpr } from "./expr.js";

// ── State ──

let port = null, reader = null, writer = null, keepReading = false;
let rxBuf = [], rxFrames = [];
let cmdLock = Promise.resolve();
let config = null;
let monitorTapTimer = null;
let exprApplyTimer = null;
let dirty = false;

// Expression source text is stored alongside config but not serialized to
// the device — only the compiled bytecode is sent. We keep the source
// in the config objects as `note_expr_src` / `velocity_expr_src` strings
// and persist them to localStorage so they survive page reloads.

// ── DOM refs (set by init) ──

let connectBanner, container, configPanel, saveBanner, toastEl;

export function init(refs) {
  connectBanner = refs.connectBanner;
  container = refs.container;
  configPanel = refs.configPanel;
  saveBanner = refs.saveBanner;
  toastEl = refs.toast;

  connectBanner.btnConnect.addEventListener("click", connect);
  saveBanner.btnSave.addEventListener("click", saveConfig);

  configPanel.btnExport.addEventListener("click", exportProject);
  configPanel.btnReset.addEventListener("click", resetConfig);
  configPanel.addEventListener("project-import", handleProjectImport);

  configPanel.addEventListener("expr-change", () => { markDirty(); debouncedApplyConfig(); });
  configPanel.addEventListener("input", () => { markDirty(); debouncedApplyConfig(); });
  configPanel.addEventListener("change", (e) => {
    markDirty();
    // If a pin dropdown changed, refresh all pin constraints immediately.
    if (e.target && e.target.classList.contains("pin-select")) {
      refreshAllPinOptions();
    }
    debouncedApplyConfig();
  });

  // Add/remove item events
  configPanel.addEventListener("item-add", handleItemAdd);
  configPanel.addEventListener("item-remove", handleItemRemove);
  configPanel.addEventListener("pin-change", () => { markDirty(); debouncedApplyConfig(); });

  if (!("serial" in navigator)) {
    connectBanner.showUnsupported();
  } else {
    navigator.serial.addEventListener("disconnect", (e) => {
      if (port && (e.target === port || e.port === port)) {
        cleanup();
        toast("Device disconnected", "info");
      }
    });
  }

  // Show config panel with defaults (disabled) on initial load
  setConnected(false);
  renderDefaultConfig();
}

// ── Toast ──

function toast(msg, type) {
  toastEl.show(msg, type);
}

// ── Connection ──

function setConnected(connected) {
  connectBanner.visible = !connected;
  container.classList.toggle("has-connect-banner", !connected);
  configPanel.disabled = !connected;
  if (!connected) {
    dirty = false;
    saveBanner.visible = false;
  }
}

function setToolbarBusy(busy) {
  connectBanner.btnConnect.disabled = busy;
  saveBanner.busy = busy;
  configPanel.projectBusy = busy;
}

function markDirty() {
  if (!dirty && config) {
    dirty = true;
    saveBanner.visible = true;
  }
}

function clearDirty() {
  dirty = false;
  saveBanner.visible = false;
}

/** Compute the set of all used pins across the config. */
function allUsedPins(cfg) {
  return {
    digital: usedDigitalPins(cfg),
    analog: usedAnalogPins(cfg),
  };
}

/**
 * Recompute used-pin sets from the current DOM and refresh the disabled
 * state on every pin <select> across all input lists and the LDR section.
 * Called after any pin dropdown changes so that other dropdowns immediately
 * reflect which pins are now available.
 */
function refreshAllPinOptions() {
  // Build a temporary config snapshot from the DOM so we can compute used pins.
  const panel = configPanel;
  const buttons = panel.buttonList.readFromDOM();
  const touch = panel.touchList.readFromDOM();
  const pots = panel.potList.readFromDOM();
  const ldr = panel.ldrSection.readFromDOM();

  const digital = new Set();
  for (const b of buttons) digital.add(b.pin);
  for (const t of touch) digital.add(t.pin);

  const analog = new Set();
  for (const p of pots) analog.add(p.pin);
  if (ldr.ldr_enabled) analog.add(ldr.ldr.pin);

  panel.buttonList.refreshPinOptions(digital);
  panel.touchList.refreshPinOptions(digital);
  panel.potList.refreshPinOptions(analog);
  panel.ldrSection.refreshPinOptions(analog);
}

/** Build a default button/touch input item with expression defaults. */
function defaultInputItem(pin, note, extras = {}) {
  const n = clamp(note, 0, 127);
  return {
    pin, note: n, velocity: 100,
    note_expr: [], velocity_expr: [],
    note_expr_src: String(n), velocity_expr_src: "100",
    ...extras,
  };
}

/** Build a default config object with firmware defaults for preview. */
function defaultConfig() {
  return {
    midi_channel: 0,
    buttons: [
      defaultInputItem(0, 60),
    ],
    touch_pads: [
      defaultInputItem(6, 48, { threshold_pct: 33 }),
    ],
    pots: [
      { pin: 26, cc: 7 },
    ],
    ldr_enabled: false,
    ldr: { pin: 28, cc: 74 },
    accel: { enabled: false, x_cc: 1, y_cc: 2, tap_note: 48, tap_velocity: 127, dead_zone_tenths: 13, smoothing_pct: 25 },
  };
}

function renderDefaultConfig() {
  const defaults = defaultConfig();
  renderConfigObj(defaults);
}

function renderConfigObj(cfg) {
  const panel = configPanel;
  const pins = allUsedPins(cfg);
  panel.midiChannel.value = cfg.midi_channel;
  panel.buttonList.render(cfg.buttons, pins.digital);
  panel.touchList.render(cfg.touch_pads, pins.digital);
  panel.potList.render(cfg.pots, pins.analog);
  panel.ldrSection.render(cfg, pins.analog);
  panel.accelSection.render(cfg);
  if (panel.pinoutGuide) panel.pinoutGuide.update(cfg);
}

async function connect() {
  try {
    port = await navigator.serial.requestPort();
    await port.open({ baudRate: 115200 });
    writer = port.writable.getWriter();
    keepReading = true;
    rxBuf = [];
    rxFrames = [];
    readLoop();
    // Hide the connect banner but keep the config panel disabled until
    // we've successfully loaded the config from the device.
    connectBanner.visible = false;
    container.classList.remove("has-connect-banner");
    const resp = await sendRequest(REQ_VERSION);
    if (resp.type === "version") {
      if (!resp.value.startsWith("midictrl")) {
        toast("Unexpected device: " + resp.value, "error");
      }
    } else {
      toast("Unexpected response", "error");
    }
    await refreshConfig();
    setConnected(true);
  } catch (e) {
    if (e.name !== "NotFoundError") {
      toast("Connection failed", "error");
    }
    cleanup();
  }
}

function cleanup() {
  keepReading = false;
  try { if (reader) { reader.cancel(); reader.releaseLock(); } } catch {}
  try { if (writer) { writer.releaseLock(); } } catch {}
  try { if (port) { port.close(); } } catch {}
  reader = null; writer = null; port = null;
  rxBuf = []; rxFrames = [];
  cmdLock = Promise.resolve();
  clearTimeout(exprApplyTimer);
  config = null;
  setConnected(false);
}

// ── Read Loop ──

async function readLoop() {
  while (keepReading && port && port.readable) {
    try {
      reader = port.readable.getReader();
      while (keepReading) {
        const { value, done } = await reader.read();
        if (done) break;
        for (let i = 0; i < value.length; i++) {
          if (value[i] === 0x00) {
            if (rxBuf.length > 0) {
              rxFrames.push(new Uint8Array(rxBuf));
              rxBuf = [];
            }
          } else {
            rxBuf.push(value[i]);
          }
        }
        drainMonitorFrames();
      }
    } catch (e) {
      // read error during shutdown is expected
    } finally {
      try { reader.releaseLock(); } catch {}
      reader = null;
    }
  }
}

// ── Monitor Frame Processing ──

function drainMonitorFrames() {
  let i = 0;
  while (i < rxFrames.length) {
    try {
      const decoded = cobsDecode(rxFrames[i]);
      const r = new PostcardReader(decoded);
      const variant = r.varint();
      if (variant === RESP_MONITOR) {
        const snap = readMonitorSnapshot(r);
        applyMonitorData(snap);
        rxFrames.splice(i, 1);
        continue;
      }
    } catch {}
    i++;
  }
}

// ── Send Request ──

function sendRequest(variantIndex, configObj) {
  const p = cmdLock.then(() => _sendRequest(variantIndex, configObj));
  cmdLock = p.catch(() => {});
  return p;
}

async function _sendRequest(variantIndex, configObj) {
  if (!writer) throw new Error("Not connected");

  rxFrames = rxFrames.filter(frame => {
    try {
      const decoded = cobsDecode(frame);
      const r = new PostcardReader(decoded);
      return r.varint() === RESP_MONITOR;
    } catch { return false; }
  });

  const frame = buildRequest(variantIndex, configObj);
  await writer.write(frame);

  const deadline = Date.now() + 3000;
  while (Date.now() < deadline) {
    await sleep(15);
    for (let i = 0; i < rxFrames.length; i++) {
      try {
        const decoded = cobsDecode(rxFrames[i]);
        const resp = parseResponse(decoded);
        if (resp && resp.type !== "monitor") {
          rxFrames.splice(i, 1);
          return resp;
        }
      } catch {
        rxFrames.splice(i, 1);
        i--;
      }
    }
  }
  throw new Error("Timeout waiting for response");
}

// ── Monitor Display ──

function updateMonitorBar(barId, valId, v) {
  const bar = document.getElementById(barId);
  const val = document.getElementById(valId);
  if (bar) bar.style.width = ((v / 127) * 100).toFixed(1) + "%";
  if (val) val.textContent = v;
}

function applyMonitorData(snap) {
  for (let i = 0; i < snap.buttons.length; i++) {
    const el = document.getElementById("monBtn" + i);
    if (el) el.classList.toggle("active", snap.buttons[i]);
  }
  for (let i = 0; i < snap.touch_pads.length; i++) {
    const el = document.getElementById("monTouch" + i);
    if (el) el.classList.toggle("active", snap.touch_pads[i]);
  }
  for (let i = 0; i < snap.pots.length; i++) {
    updateMonitorBar("monPotBar" + i, "monPotVal" + i, snap.pots[i]);
  }
  updateMonitorBar("monLdrBar", "monLdrVal", snap.ldr);
  updateMonitorBar("monAccelXBar", "monAccelXVal", snap.accel_x);
  updateMonitorBar("monAccelYBar", "monAccelYVal", snap.accel_y);
  if (snap.accel_tap) {
    const el = document.getElementById("monAccelTap");
    if (el) {
      el.classList.add("active");
      clearTimeout(monitorTapTimer);
      monitorTapTimer = setTimeout(() => el.classList.remove("active"), 200);
    }
  }
}

// ── Expression Source Persistence ──
// The device only stores bytecode, not expression source text. We keep the
// human-readable source in localStorage keyed by button/touch index so users
// can edit it after reconnecting.

/** Ensure note_expr_src / velocity_expr_src are initialized on each item. */
function initExprSources(items) {
  items.forEach(item => {
    item.note_expr_src = item.note_expr_src || "";
    item.velocity_expr_src = item.velocity_expr_src || "";
  });
}

function saveExprSources() {
  if (!config) return;
  const data = {
    buttons: config.buttons.map(b => ({ pin: b.pin, note: b.note_expr_src || "", vel: b.velocity_expr_src || "" })),
    touch: config.touch_pads.map(t => ({ pin: t.pin, note: t.note_expr_src || "", vel: t.velocity_expr_src || "" })),
  };
  try { localStorage.setItem("picomidi_expr", JSON.stringify(data)); } catch {}
}

function loadExprSources() {
  try {
    const data = JSON.parse(localStorage.getItem("picomidi_expr") || "{}");
    // Match stored sources to config items by pin number (more robust than index)
    const applyData = (items, stored) => {
      if (!items || !stored) return;
      items.forEach(item => {
        const match = stored.find(s => s.pin === item.pin);
        if (match) {
          item.note_expr_src = match.note || "";
          item.velocity_expr_src = match.vel || "";
        }
      });
    };
    applyData(config.buttons, data.buttons);
    applyData(config.touch_pads, data.touch);
  } catch {}
}

// ── Add / Remove Items ──

function handleItemAdd(e) {
  if (!config) return;
  const type = e.detail.type;

  // Read current UI state into config first
  readConfigFromUI();
  if (!config) return;

  if (type === "button") {
    if (config.buttons.length >= MAX_DIGITAL_INPUTS) return;
    const used = usedDigitalPins(config);
    const pin = nextAvailableDigitalPin(used);
    config.buttons.push(defaultInputItem(pin, 60 + config.buttons.length));
  } else if (type === "touch") {
    if (config.touch_pads.length >= MAX_DIGITAL_INPUTS) return;
    const used = usedDigitalPins(config);
    const pin = nextAvailableDigitalPin(used);
    config.touch_pads.push(defaultInputItem(pin, 48 + config.touch_pads.length, { threshold_pct: 33 }));
  } else if (type === "pot") {
    if (config.pots.length >= MAX_ANALOG_INPUTS) return;
    const used = usedAnalogPins(config);
    const pin = nextAvailableAnalogPin(used);
    config.pots.push({ pin, cc: 1 });
  }

  renderConfigObj(config);
  markDirty();
  debouncedApplyConfig();
}

function handleItemRemove(e) {
  if (!config) return;
  const { type, index } = e.detail;

  // Read current UI state into config first
  readConfigFromUI();
  if (!config) return;

  if (type === "button" && config.buttons.length > 0) {
    config.buttons.splice(index, 1);
  } else if (type === "touch" && config.touch_pads.length > 0) {
    config.touch_pads.splice(index, 1);
  } else if (type === "pot" && config.pots.length > 0) {
    config.pots.splice(index, 1);
  }

  renderConfigObj(config);
  markDirty();
  debouncedApplyConfig();
}

// ── Config Operations ──

/** Format a protocol response as an error message. */
function responseError(resp) {
  return resp.type === "error" ? resp.message : "Unexpected response: " + resp.type;
}

/** Run an async operation with the toolbar in busy state. */
async function withBusy(fn) {
  setToolbarBusy(true);
  try { return await fn(); }
  finally { setToolbarBusy(false); }
}

async function refreshConfig() {
  await withBusy(async () => {
    try {
      const resp = await sendRequest(REQ_GET_CONFIG);
      if (resp.type === "config") {
        config = resp.value;
        // Initialize expr source strings (empty by default from device)
        initExprSources(config.buttons);
        initExprSources(config.touch_pads);
        loadExprSources();
        renderConfigObj(config);
        clearDirty();
        toast("Config loaded", "success");
      } else {
        throw new Error(responseError(resp));
      }
    } catch (e) {
      toast("Failed to load config", "error");
      throw e;
    }
  });
}

/**
 * Extract a static fallback value from an expression source string.
 * If the expression is a plain integer (e.g. "60"), return that number.
 * Otherwise return the provided default.
 */
function staticFromExpr(src, fallback) {
  return parseStaticInt(src) ?? fallback;
}

/**
 * Compile expression fields on a button/touch item from the DOM.
 * Returns the compiled item, or { error } on failure.
 */
function compileInputItem(item, label, extraFields = {}) {
  const noteResult = compileExpr(item.note_expr_src);
  const velResult = compileExpr(item.velocity_expr_src);
  if (noteResult.error) return { error: `${label} note expr: ${noteResult.error}` };
  if (velResult.error) return { error: `${label} velocity expr: ${velResult.error}` };
  return {
    pin: item.pin,
    note: clamp(staticFromExpr(item.note_expr_src, 60), 0, 127),
    velocity: clamp(staticFromExpr(item.velocity_expr_src, 100), 1, 127),
    note_expr: Array.from(noteResult.code),
    velocity_expr: Array.from(velResult.code),
    note_expr_src: item.note_expr_src,
    velocity_expr_src: item.velocity_expr_src,
    ...extraFields,
  };
}

function readConfigFromUI() {
  if (!config) return null;
  const panel = configPanel;
  config.midi_channel = clamp(num(panel.midiChannel.value, 0), 0, 15);

  // Buttons — compile expression text to bytecode
  config.buttons = panel.buttonList.readFromDOM().map(b =>
    compileInputItem(b, "Button")
  );
  for (const b of config.buttons) {
    if (b.error) { toast(b.error, "error"); return null; }
  }

  // Touch pads — compile expression text to bytecode
  config.touch_pads = panel.touchList.readFromDOM().map(t =>
    compileInputItem(t, "Touch", { threshold_pct: clamp(t.threshold_pct, 1, 255) })
  );
  for (const t of config.touch_pads) {
    if (t.error) { toast(t.error, "error"); return null; }
  }

  // Pots
  config.pots = panel.potList.readFromDOM().map(p => ({
    pin: p.pin,
    cc: clamp(p.cc, 0, 127),
  }));

  // LDR & Accel — delegate to their components
  const ldrData = panel.ldrSection.readFromDOM();
  config.ldr_enabled = ldrData.ldr_enabled;
  config.ldr = ldrData.ldr;

  config.accel = panel.accelSection.readFromDOM();

  return config;
}

async function applyConfig() {
  try {
    const cfg = readConfigFromUI();
    if (!cfg) return false;
    saveExprSources();
    if (configPanel.pinoutGuide) configPanel.pinoutGuide.update(cfg);
    const resp = await sendRequest(REQ_PUT_CONFIG, cfg);
    if (resp.type === "ok") return true;
    throw new Error(responseError(resp));
  } catch (e) {
    toast("Apply failed: " + e.message, "error");
    return false;
  }
}

function debouncedApplyConfig() {
  clearTimeout(exprApplyTimer);
  exprApplyTimer = setTimeout(() => applyConfig(), 150);
}

async function saveConfig() {
  await withBusy(async () => {
    try {
      if (!await applyConfig()) return;
      const resp = await sendRequest(REQ_SAVE);
      if (resp.type === "ok") {
        clearDirty();
        toast("Saved to flash", "success");
      }
      else toast("Save failed: " + (resp.message || resp.type), "error");
    } catch (e) {
      toast("Save failed", "error");
    }
  });
}

async function resetConfig() {
  if (!confirm("Reset all config to factory defaults? (in RAM only, use Save to persist)")) return;
  await withBusy(async () => {
    try {
      const resp = await sendRequest(REQ_RESET);
      if (resp.type === "ok") {
        try { localStorage.removeItem("picomidi_expr"); } catch {}
        await refreshConfig();
        markDirty();
        toast("Defaults restored", "info");
      }
    } catch (e) {
      toast("Reset failed", "error");
    }
  });
}

// ── Project Export / Import ──

/** Normalize a button/touch item to a clean JSON-serializable shape. */
function normalizeInputItem(item, extraFields = {}) {
  return {
    pin: item.pin,
    note: clamp(item.note, 0, 127),
    velocity: clamp(item.velocity, 1, 127),
    note_expr: Array.from(item.note_expr),
    velocity_expr: Array.from(item.velocity_expr),
    note_expr_src: item.note_expr_src || "",
    velocity_expr_src: item.velocity_expr_src || "",
    ...extraFields,
  };
}

function exportProject() {
  // Read current UI state into config (so unsaved edits are captured)
  const cfg = readConfigFromUI();
  if (!cfg) return;

  // Build a clean JSON-serializable project object
  const project = {
    _format: "pico-midi-project",
    _version: 2,
    midi_channel: cfg.midi_channel,
    buttons: cfg.buttons.map(b => normalizeInputItem(b)),
    touch_pads: cfg.touch_pads.map(t => normalizeInputItem(t, { threshold_pct: t.threshold_pct })),
    pots: cfg.pots.map(p => ({ pin: p.pin, cc: p.cc })),
    ldr_enabled: cfg.ldr_enabled,
    ldr: { pin: cfg.ldr.pin, cc: cfg.ldr.cc },
    accel: { ...cfg.accel },
  };

  const json = JSON.stringify(project, null, 2);
  const blob = new Blob([json], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = "pico-midi-project.json";
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
  toast("Project exported", "success");
}

async function handleProjectImport(e) {
  const file = e.detail.file;
  let project;
  try {
    const text = await file.text();
    project = JSON.parse(text);
  } catch {
    toast("Invalid JSON file", "error");
    return;
  }

  // Validate the project structure
  if (!validateProject(project)) {
    toast("Invalid project file", "error");
    return;
  }

  await withBusy(async () => {
    try {
      // Apply imported config to the in-memory config object
      config = {
        midi_channel: clamp(project.midi_channel, 0, 15),
        buttons: project.buttons.map(b => normalizeInputItem(b)),
        touch_pads: project.touch_pads.map(t => normalizeInputItem(t, { threshold_pct: clamp(t.threshold_pct, 1, 255) })),
        pots: project.pots.map(p => ({ pin: clamp(p.pin, 0, 29), cc: clamp(p.cc, 0, 127) })),
        ldr_enabled: !!project.ldr_enabled,
        ldr: { pin: clamp(project.ldr.pin, 0, 29), cc: clamp(project.ldr.cc, 0, 127) },
        accel: {
          enabled: !!project.accel.enabled,
          x_cc: clamp(project.accel.x_cc, 0, 127),
          y_cc: clamp(project.accel.y_cc, 0, 127),
          tap_note: clamp(project.accel.tap_note, 0, 127),
          tap_velocity: clamp(project.accel.tap_velocity, 1, 127),
          dead_zone_tenths: clamp(project.accel.dead_zone_tenths, 0, 255),
          smoothing_pct: clamp(project.accel.smoothing_pct, 0, 100),
        },
      };

      // Render the imported config to the UI
      renderConfigObj(config);

      // Save expression sources to localStorage
      saveExprSources();

      // Send the config to the device
      const resp = await sendRequest(REQ_PUT_CONFIG, config);
      if (resp.type === "ok") {
        markDirty();
        toast("Project imported", "success");
      } else {
        throw new Error(responseError(resp));
      }
    } catch (e) {
      toast("Import failed: " + e.message, "error");
    }
  });
}

function validateProject(p) {
  if (!p || typeof p !== "object") return false;
  if (p._format !== "pico-midi-project") return false;
  if (typeof p.midi_channel !== "number") return false;

  // Variable-length arrays: just check they exist and are within bounds
  if (!Array.isArray(p.buttons) || p.buttons.length > MAX_DIGITAL_INPUTS) return false;
  if (!Array.isArray(p.touch_pads) || p.touch_pads.length > MAX_DIGITAL_INPUTS) return false;
  if (!Array.isArray(p.pots) || p.pots.length > MAX_ANALOG_INPUTS) return false;

  if (!p.ldr || typeof p.ldr.cc !== "number") return false;
  if (!p.accel || typeof p.accel.enabled !== "boolean") return false;

  for (const b of p.buttons) {
    if (typeof b.note !== "number" || typeof b.velocity !== "number") return false;
    if (!Array.isArray(b.note_expr) || !Array.isArray(b.velocity_expr)) return false;
    if (typeof b.pin !== "number") return false;
  }
  for (const t of p.touch_pads) {
    if (typeof t.note !== "number" || typeof t.velocity !== "number") return false;
    if (typeof t.threshold_pct !== "number") return false;
    if (!Array.isArray(t.note_expr) || !Array.isArray(t.velocity_expr)) return false;
    if (typeof t.pin !== "number") return false;
  }
  for (const p2 of p.pots) {
    if (typeof p2.cc !== "number") return false;
    if (typeof p2.pin !== "number") return false;
  }
  return true;
}
