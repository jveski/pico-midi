import {
  cobsDecode, buildRequest, parseResponse, readMonitorSnapshot,
  PostcardReader,
  REQ_VERSION, REQ_GET_CONFIG, REQ_PUT_CONFIG, REQ_SAVE, REQ_RESET,
  RESP_MONITOR, MAX_DIGITAL_INPUTS, MAX_ANALOG_INPUTS,
} from "./protocol.js";
import { sleep, num, clamp, parseStaticInt, usedDigitalPins, usedAnalogPins, nextAvailableDigitalPin, nextAvailableAnalogPin, isValidInputItem } from "./helpers.js";
import { compileExpr } from "./expr.js";

let port = null, reader = null, writer = null, keepReading = false;
let rxBuf = [], rxFrames = [];
let cmdLock = Promise.resolve();
let config = null;
let monitorTapTimer = null;
let exprApplyTimer = null;
let dirty = false;

let toolbar, configPanel, modalPinout, exprEditor, toastEl;

export function init(refs) {
  toolbar = refs.toolbar;
  configPanel = refs.configPanel;
  modalPinout = refs.modalPinout;
  exprEditor = refs.exprEditor;
  toastEl = refs.toast;

  toolbar.btnConnect.addEventListener("click", handleConnectClick);
  toolbar.btnSave.addEventListener("click", saveConfig);

  // Modal open buttons
  toolbar.btnPinout.addEventListener("click", () => modalPinout.toggle());

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

  // Expression editor: open modal when edit button is clicked
  configPanel.addEventListener("edit-expr", handleEditExpr);

  if (!("serial" in navigator)) {
    toolbar.showUnsupported();
  } else {
    navigator.serial.addEventListener("disconnect", async (e) => {
      if (port && (e.target === port || e.port === port)) {
        await cleanup();
        toast("Device disconnected", "info");
      }
    });
  }

  // Show config panel with defaults (disabled) on initial load
  setConnected(false);
  renderDefaultConfig();
}

function toast(msg, type) {
  toastEl.show(msg, type);
}

function setConnected(connected) {
  toolbar.connected = connected;
  toolbar.statusEl.textContent = connected ? "Connected" : "No device";
  toolbar.statusEl.classList.toggle("unsupported", false);
  configPanel.disabled = !connected;
  if (!connected) {
    dirty = false;
    toolbar.saveVisible = false;
  }
}

function setToolbarBusy(busy) {
  toolbar.btnConnect.disabled = busy;
  toolbar.saveBusy = busy;
  configPanel.projectBusy = busy;
}

function markDirty() {
  if (!dirty && config) {
    dirty = true;
    toolbar.saveVisible = true;
  }
}

function clearDirty() {
  dirty = false;
  toolbar.saveVisible = false;
}

function allUsedPins(cfg) {
  return {
    digital: usedDigitalPins(cfg),
    analog: usedAnalogPins(cfg),
  };
}

function refreshAllPinOptions() {
  // Build a temporary config snapshot from the DOM so we can compute used pins.
  const panel = configPanel;
  const tempCfg = {
    buttons: panel.buttonList.readFromDOM(),
    touch_pads: panel.touchList.readFromDOM(),
    pots: panel.potList.readFromDOM(),
    ...panel.ldrSection.readFromDOM(),
  };
  const digital = usedDigitalPins(tempCfg);
  const analog = usedAnalogPins(tempCfg);

  panel.buttonList.refreshPinOptions(digital);
  panel.touchList.refreshPinOptions(digital);
  panel.potList.refreshPinOptions(analog);
  panel.ldrSection.refreshPinOptions(analog);
}

function defaultInputItem(pin, note, extras = {}) {
  const n = clamp(note, 0, 127);
  return {
    pin, note: n, velocity: 100,
    note_expr: [], velocity_expr: [],
    note_expr_src: String(n), velocity_expr_src: "100",
    ...extras,
  };
}

function defaultConfig() {
  return {
    midi_channel: 0,
    buttons: [
      defaultInputItem(0, 60),
    ],
    touch_pads: [
      defaultInputItem(6, 48, { threshold_pct: 25 }),
    ],
    pots: [
      { pin: 26, cc: 7 },
    ],
    ldr_enabled: false,
    ldr: { pin: 28, cc: 74 },
    accel: { enabled: false, chip: 0, x_cc: 1, y_cc: 2, tap_note: 48, tap_velocity: 127, dead_zone_tenths: 13, smoothing_pct: 25 },
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
  // Update pinout guide in its modal
  const pinout = modalPinout ? modalPinout.querySelector("pinout-guide") : null;
  if (pinout) pinout.update(cfg);
}

async function handleConnectClick() {
  toolbar.btnConnect.disabled = true;
  try {
    if (port) {
      await cleanup();
      toast("Disconnected", "info");
    } else {
      await connect();
    }
  } finally {
    toolbar.btnConnect.disabled = false;
  }
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
    // Show connecting state
    toolbar.statusEl.textContent = "Connecting...";
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
    await cleanup();
  }
}

async function cleanup() {
  keepReading = false;
  try { if (reader) { await reader.cancel(); reader.releaseLock(); } } catch {}
  try { if (writer) { writer.releaseLock(); } } catch {}
  try { if (port) { await port.close(); } } catch {}
  reader = null; writer = null; port = null;
  rxBuf = []; rxFrames = [];
  cmdLock = Promise.resolve();
  clearTimeout(exprApplyTimer);
  config = null;
  setConnected(false);
}

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

function updateMonitorBar(barId, valId, v) {
  const bar = document.getElementById(barId);
  const val = document.getElementById(valId);
  if (bar) bar.style.width = ((v / 127) * 100).toFixed(1) + "%";
  if (val) val.textContent = v;
}

function toggleMonitorIndicators(prefix, values) {
  for (let i = 0; i < values.length; i++) {
    const el = document.getElementById(prefix + i);
    if (el) el.classList.toggle("active", values[i]);
  }
}

function applyMonitorData(snap) {
  toggleMonitorIndicators("monBtn", snap.buttons);
  toggleMonitorIndicators("monTouch", snap.touch_pads);
  // Touch pad telemetry bars
  for (let i = 0; i < snap.touch_pads.length; i++) {
    const bar = document.getElementById("monTouchBar" + i);
    const thrMarker = document.getElementById("monTouchThr" + i);
    const valEl = document.getElementById("monTouchVal" + i);
    if (!bar) continue;
    const filtered = snap.touch_filtered[i] || 0;
    const baseline = snap.touch_baseline[i] || 0;
    const threshold = snap.touch_threshold[i] || 0;
    // Scale: bar range is 0 to max(threshold * 2, filtered, 1)
    const ceil = Math.max(threshold * 2, filtered, baseline + 100, 1);
    bar.style.width = ((filtered / ceil) * 100).toFixed(1) + "%";
    bar.classList.toggle("touch-active", snap.touch_pads[i]);
    if (thrMarker) {
      thrMarker.style.left = ((threshold / ceil) * 100).toFixed(1) + "%";
    }
    if (valEl) {
      valEl.textContent = filtered;
      valEl.title = `filtered: ${filtered}  baseline: ${baseline}  threshold: ${threshold}  (values in CPU cycles / 4)`;
    }
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

function initExprSources(items) {
  items.forEach(item => {
    item.note_expr_src = item.note_expr_src || "";
    item.velocity_expr_src = item.velocity_expr_src || "";
  });
}

function saveExprSources() {
  if (!config) return;
  const summary = item => ({ pin: item.pin, note: item.note_expr_src || "", vel: item.velocity_expr_src || "" });
  const data = { buttons: config.buttons.map(summary), touch: config.touch_pads.map(summary) };
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

const TYPE_KEYS = { button: "buttons", touch: "touch_pads", pot: "pots" };

function withConfigMutation(e, mutate) {
  if (!config) return;
  const updated = readConfigFromUI();
  if (!updated) return;
  if (mutate(e.detail) === false) return;
  renderConfigObj(config);
  markDirty();
  debouncedApplyConfig();
}

function handleItemAdd(e) {
  withConfigMutation(e, ({ type }) => {
    if (type === "button" || type === "touch") {
      const arr = type === "button" ? config.buttons : config.touch_pads;
      if (arr.length >= MAX_DIGITAL_INPUTS) return false;
      const pin = nextAvailableDigitalPin(usedDigitalPins(config));
      const baseNote = type === "button" ? 60 : 48;
      const extras = type === "touch" ? { threshold_pct: 25 } : {};
      arr.push(defaultInputItem(pin, baseNote + arr.length, extras));
    } else if (type === "pot") {
      if (config.pots.length >= MAX_ANALOG_INPUTS) return false;
      config.pots.push({ pin: nextAvailableAnalogPin(usedAnalogPins(config)), cc: 1 });
    }
  });
}

function handleItemRemove(e) {
  withConfigMutation(e, ({ type, index }) => {
    const key = TYPE_KEYS[type];
    if (key && config[key].length > 0) config[key].splice(index, 1);
  });
}

function handleEditExpr(e) {
  const { type, index, field, input } = e.detail;
  const isNote = field === "note";
  const label = type === "button" ? "Button" : "Touch Pad";
  const fieldLabel = isNote ? "Note" : "Velocity";
  const title = `${label} #${index + 1} — ${fieldLabel}`;

  exprEditor.open({
    title,
    value: input.value,
    isNote,
    onApply(newValue) {
      input.value = newValue;
      // Trigger validation and config update
      input.dispatchEvent(new Event("input", { bubbles: true }));
    },
  });
}

function responseError(resp) {
  return resp.type === "error" ? resp.message : "Unexpected response: " + resp.type;
}

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

function staticFromExpr(src, fallback) {
  return parseStaticInt(src) ?? fallback;
}

function normalizeInputItem(item, extraFields = {}) {
  return {
    pin: item.pin,
    note: clamp(item.note ?? 60, 0, 127),
    velocity: clamp(item.velocity ?? 100, 1, 127),
    note_expr: Array.from(item.note_expr || []),
    velocity_expr: Array.from(item.velocity_expr || []),
    note_expr_src: item.note_expr_src || "",
    velocity_expr_src: item.velocity_expr_src || "",
    ...extraFields,
  };
}

function compileInputItem(item, label, extraFields = {}) {
  const noteResult = compileExpr(item.note_expr_src);
  const velResult = compileExpr(item.velocity_expr_src);
  if (noteResult.error) return { error: `${label} note expr: ${noteResult.error}` };
  if (velResult.error) return { error: `${label} velocity expr: ${velResult.error}` };
  return normalizeInputItem({
    ...item,
    note: staticFromExpr(item.note_expr_src, 60),
    velocity: staticFromExpr(item.velocity_expr_src, 100),
    note_expr: noteResult.code,
    velocity_expr: velResult.code,
  }, extraFields);
}

function readConfigFromUI() {
  if (!config) return null;
  const panel = configPanel;

  // Buttons — compile expression text to bytecode
  const buttons = panel.buttonList.readFromDOM().map(b =>
    compileInputItem(b, "Button")
  );
  for (const b of buttons) {
    if (b.error) { toast(b.error, "error"); return null; }
  }

  // Touch pads — compile expression text to bytecode
  const touch_pads = panel.touchList.readFromDOM().map(t =>
    compileInputItem(t, "Touch", { threshold_pct: clamp(t.threshold_pct, 1, 255) })
  );
  for (const t of touch_pads) {
    if (t.error) { toast(t.error, "error"); return null; }
  }

  // All compilations succeeded — safe to mutate config
  config.midi_channel = clamp(num(panel.midiChannel.value, 0), 0, 15);
  config.buttons = buttons;
  config.touch_pads = touch_pads;

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
    const pinout = modalPinout ? modalPinout.querySelector("pinout-guide") : null;
    if (pinout) pinout.update(cfg);
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

function exportProject() {
  // Read current UI state into config (so unsaved edits are captured)
  const cfg = readConfigFromUI();
  if (!cfg) return;

  // Build a clean JSON-serializable project object
  const project = {
    _format: "pico-midi-project",
    _version: 3,
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
          chip: clamp(project.accel.chip ?? 0, 0, 2),
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

  if (!p.buttons.every(isValidInputItem)) return false;
  if (!p.touch_pads.every(t => isValidInputItem(t) && typeof t.threshold_pct === "number")) return false;
  for (const p2 of p.pots) {
    if (typeof p2.cc !== "number") return false;
    if (typeof p2.pin !== "number") return false;
  }
  return true;
}
