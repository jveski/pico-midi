import {
  cobsDecode, buildRequest, parseResponse, readMonitorSnapshot,
  PostcardReader,
  REQ_VERSION, REQ_GET_CONFIG, REQ_PUT_CONFIG, REQ_SAVE, REQ_RESET,
  RESP_MONITOR, MAX_BUTTONS, MAX_TOUCH_PADS, MAX_POTS,
} from "./protocol.js";
import { sleep, num, clamp } from "./helpers.js";
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

let statusBar, toolbar, configPanel, saveBanner, emptyState, toastEl;

export function init(refs) {
  statusBar = refs.statusBar;
  toolbar = refs.toolbar;
  configPanel = refs.configPanel;
  saveBanner = refs.saveBanner;
  emptyState = refs.emptyState;
  toastEl = refs.toast;

  toolbar.btnConnect.addEventListener("click", connect);
  saveBanner.btnSave.addEventListener("click", saveConfig);

  configPanel.btnExport.addEventListener("click", exportProject);
  configPanel.btnReset.addEventListener("click", resetConfig);
  configPanel.addEventListener("project-import", handleProjectImport);

  configPanel.addEventListener("expr-change", () => { markDirty(); debouncedApplyConfig(); });
  configPanel.addEventListener("input", markDirty);
  configPanel.addEventListener("change", markDirty);

  if (!("serial" in navigator)) {
    document.getElementById("unsupported").style.display = "block";
    toolbar.btnConnect.disabled = true;
  } else {
    navigator.serial.addEventListener("disconnect", (e) => {
      if (port && (e.target === port || e.port === port)) {
        cleanup();
        toast("Device disconnected", "info");
      }
    });
  }
}

// ── Toast ──

function toast(msg, type) {
  toastEl.show(msg, type);
}

// ── Connection ──

function setConnected(connected) {
  statusBar.connected = connected;
  toolbar.connected = connected;
  configPanel.style.display = connected ? "" : "none";
  emptyState.style.display = connected ? "none" : "";
  if (!connected) {
    dirty = false;
    saveBanner.visible = false;
  }
}

function setToolbarBusy(busy) {
  toolbar.busy = busy;
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

async function connect() {
  try {
    port = await navigator.serial.requestPort();
    await port.open({ baudRate: 115200 });
    writer = port.writable.getWriter();
    keepReading = true;
    rxBuf = [];
    rxFrames = [];
    readLoop();
    setConnected(true);
    const resp = await sendRequest(REQ_VERSION);
    if (resp.type === "version") {
      if (!resp.value.startsWith("midictrl")) {
        toast("Unexpected device: " + resp.value, "error");
      } else {
        statusBar.text = resp.value;
      }
    } else {
      toast("Unexpected response", "error");
    }
    await refreshConfig();
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
  for (let i = 0; i < snap.buttons.length && i < MAX_BUTTONS; i++) {
    const el = document.getElementById("monBtn" + i);
    if (el) el.classList.toggle("active", snap.buttons[i]);
  }
  for (let i = 0; i < snap.touch_pads.length && i < MAX_TOUCH_PADS; i++) {
    const el = document.getElementById("monTouch" + i);
    if (el) el.classList.toggle("active", snap.touch_pads[i]);
  }
  for (let i = 0; i < snap.pots.length && i < MAX_POTS; i++) {
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

function saveExprSources() {
  if (!config) return;
  const data = {
    buttons: config.buttons.map(b => ({ note: b.note_expr_src || "", vel: b.velocity_expr_src || "" })),
    touch: config.touch_pads.map(t => ({ note: t.note_expr_src || "", vel: t.velocity_expr_src || "" })),
  };
  try { localStorage.setItem("picomidi_expr", JSON.stringify(data)); } catch {}
}

function loadExprSources() {
  try {
    const data = JSON.parse(localStorage.getItem("picomidi_expr") || "{}");
    if (config && data.buttons) {
      config.buttons.forEach((b, i) => {
        if (data.buttons[i]) {
          b.note_expr_src = data.buttons[i].note || "";
          b.velocity_expr_src = data.buttons[i].vel || "";
        }
      });
    }
    if (config && data.touch) {
      config.touch_pads.forEach((t, i) => {
        if (data.touch[i]) {
          t.note_expr_src = data.touch[i].note || "";
          t.velocity_expr_src = data.touch[i].vel || "";
        }
      });
    }
  } catch {}
}

// ── Config Operations ──

async function refreshConfig() {
  setToolbarBusy(true);
  try {
    const resp = await sendRequest(REQ_GET_CONFIG);
    if (resp.type === "config") {
      config = resp.value;
      // Initialize expr source strings (empty by default from device)
      config.buttons.forEach(b => {
        b.note_expr_src = b.note_expr_src || "";
        b.velocity_expr_src = b.velocity_expr_src || "";
      });
      config.touch_pads.forEach(t => {
        t.note_expr_src = t.note_expr_src || "";
        t.velocity_expr_src = t.velocity_expr_src || "";
      });
      loadExprSources();
      renderConfig();
      clearDirty();
      toast("Config loaded", "success");
    } else if (resp.type === "error") {
      throw new Error(resp.message);
    } else {
      throw new Error("Unexpected response: " + resp.type);
    }
  } catch (e) {
    toast("Failed to load config", "error");
  } finally {
    setToolbarBusy(false);
  }
}

function renderConfig() {
  if (!config) return;
  const panel = configPanel;
  panel.midiChannel.value = config.midi_channel;
  panel.buttonList.render(config.buttons);
  panel.touchList.render(config.touch_pads);
  panel.potList.render(config.pots);
  panel.ldrSection.render(config);
  panel.accelSection.render(config);
}

/**
 * Extract a static fallback value from an expression source string.
 * If the expression is a plain integer (e.g. "60"), return that number.
 * Otherwise return the provided default.
 */
function staticFromExpr(src, fallback) {
  const v = parseInt(src, 10);
  return (String(v) === (src || "").trim()) ? v : fallback;
}

function readConfigFromUI() {
  if (!config) return null;
  const panel = configPanel;
  config.midi_channel = clamp(num(panel.midiChannel.value, 0), 0, 15);

  // Buttons — compile expression text to bytecode; extract static fallback
  config.buttons = panel.buttonList.readFromDOM().map(b => {
    const noteResult = compileExpr(b.note_expr_src);
    const velResult = compileExpr(b.velocity_expr_src);
    if (noteResult.error) return { error: `Button note expr: ${noteResult.error}` };
    if (velResult.error) return { error: `Button velocity expr: ${velResult.error}` };
    return {
      note: clamp(staticFromExpr(b.note_expr_src, 60), 0, 127),
      velocity: clamp(staticFromExpr(b.velocity_expr_src, 100), 1, 127),
      note_expr: Array.from(noteResult.code),
      velocity_expr: Array.from(velResult.code),
      note_expr_src: b.note_expr_src,
      velocity_expr_src: b.velocity_expr_src,
    };
  });

  // Check for compilation errors
  for (const b of config.buttons) {
    if (b.error) { toast(b.error, "error"); return null; }
  }

  // Touch pads — compile expression text to bytecode; extract static fallback
  config.touch_pads = panel.touchList.readFromDOM().map(t => {
    const noteResult = compileExpr(t.note_expr_src);
    const velResult = compileExpr(t.velocity_expr_src);
    if (noteResult.error) return { error: `Touch note expr: ${noteResult.error}` };
    if (velResult.error) return { error: `Touch velocity expr: ${velResult.error}` };
    return {
      note: clamp(staticFromExpr(t.note_expr_src, 60), 0, 127),
      velocity: clamp(staticFromExpr(t.velocity_expr_src, 100), 1, 127),
      threshold_pct: clamp(t.threshold_pct, 1, 255),
      note_expr: Array.from(noteResult.code),
      velocity_expr: Array.from(velResult.code),
      note_expr_src: t.note_expr_src,
      velocity_expr_src: t.velocity_expr_src,
    };
  });

  for (const t of config.touch_pads) {
    if (t.error) { toast(t.error, "error"); return null; }
  }

  // Pots
  config.pots = panel.potList.readFromDOM().map(p => ({
    cc: clamp(p.cc, 0, 127),
  }));

  config.ldr_enabled = document.getElementById("ldrEnabled").checked;
  config.ldr = {
    cc: clamp(num(document.getElementById("ldrCc").value, 0), 0, 127),
  };

  config.accel = {
    enabled: document.getElementById("accelEnabled").checked,
    x_cc: clamp(num(document.getElementById("accelXCc").value, 0), 0, 127),
    y_cc: clamp(num(document.getElementById("accelYCc").value, 0), 0, 127),
    tap_note: clamp(num(document.getElementById("accelTapNote").value, 0), 0, 127),
    tap_velocity: clamp(num(document.getElementById("accelTapVel").value, 1), 1, 127),
    dead_zone_tenths: clamp(num(document.getElementById("accelDeadZone").value, 0), 0, 255),
    smoothing_pct: clamp(num(document.getElementById("accelSmoothing").value, 0), 0, 100),
  };

  return config;
}

async function applyConfig() {
  try {
    const cfg = readConfigFromUI();
    if (!cfg) return false;
    saveExprSources();
    const resp = await sendRequest(REQ_PUT_CONFIG, cfg);
    if (resp.type === "ok") return true;
    throw new Error(resp.type === "error" ? resp.message : "Unexpected: " + resp.type);
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
  setToolbarBusy(true);
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
  } finally {
    setToolbarBusy(false);
  }
}

async function resetConfig() {
  if (!confirm("Reset all config to factory defaults? (in RAM only, use Save to persist)")) return;
  setToolbarBusy(true);
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
  } finally {
    setToolbarBusy(false);
  }
}

// ── Project Export / Import ──

function exportProject() {
  // Read current UI state into config (so unsaved edits are captured)
  const cfg = readConfigFromUI();
  if (!cfg) return;

  // Build a clean JSON-serializable project object
  const project = {
    _format: "pico-midi-project",
    _version: 1,
    midi_channel: cfg.midi_channel,
    buttons: cfg.buttons.map(b => ({
      note: b.note,
      velocity: b.velocity,
      note_expr: Array.from(b.note_expr),
      velocity_expr: Array.from(b.velocity_expr),
      note_expr_src: b.note_expr_src || "",
      velocity_expr_src: b.velocity_expr_src || "",
    })),
    touch_pads: cfg.touch_pads.map(t => ({
      note: t.note,
      velocity: t.velocity,
      threshold_pct: t.threshold_pct,
      note_expr: Array.from(t.note_expr),
      velocity_expr: Array.from(t.velocity_expr),
      note_expr_src: t.note_expr_src || "",
      velocity_expr_src: t.velocity_expr_src || "",
    })),
    pots: cfg.pots.map(p => ({ cc: p.cc })),
    ldr_enabled: cfg.ldr_enabled,
    ldr: { cc: cfg.ldr.cc },
    accel: {
      enabled: cfg.accel.enabled,
      x_cc: cfg.accel.x_cc,
      y_cc: cfg.accel.y_cc,
      tap_note: cfg.accel.tap_note,
      tap_velocity: cfg.accel.tap_velocity,
      dead_zone_tenths: cfg.accel.dead_zone_tenths,
      smoothing_pct: cfg.accel.smoothing_pct,
    },
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

  setToolbarBusy(true);
  try {
    // Apply imported config to the in-memory config object
    config = {
      midi_channel: clamp(project.midi_channel, 0, 15),
      buttons: project.buttons.map(b => ({
        note: clamp(b.note, 0, 127),
        velocity: clamp(b.velocity, 1, 127),
        note_expr: Array.from(b.note_expr),
        velocity_expr: Array.from(b.velocity_expr),
        note_expr_src: b.note_expr_src || "",
        velocity_expr_src: b.velocity_expr_src || "",
      })),
      touch_pads: project.touch_pads.map(t => ({
        note: clamp(t.note, 0, 127),
        velocity: clamp(t.velocity, 1, 127),
        threshold_pct: clamp(t.threshold_pct, 1, 255),
        note_expr: Array.from(t.note_expr),
        velocity_expr: Array.from(t.velocity_expr),
        note_expr_src: t.note_expr_src || "",
        velocity_expr_src: t.velocity_expr_src || "",
      })),
      pots: project.pots.map(p => ({ cc: clamp(p.cc, 0, 127) })),
      ldr_enabled: !!project.ldr_enabled,
      ldr: { cc: clamp(project.ldr.cc, 0, 127) },
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
    renderConfig();

    // Save expression sources to localStorage
    saveExprSources();

    // Send the config to the device
    const resp = await sendRequest(REQ_PUT_CONFIG, config);
    if (resp.type === "ok") {
      markDirty();
      toast("Project imported", "success");
    } else {
      throw new Error(resp.type === "error" ? resp.message : "Unexpected: " + resp.type);
    }
  } catch (e) {
    toast("Import failed: " + e.message, "error");
  } finally {
    setToolbarBusy(false);
  }
}

function validateProject(p) {
  if (!p || typeof p !== "object") return false;
  if (p._format !== "pico-midi-project") return false;
  if (typeof p.midi_channel !== "number") return false;
  if (!Array.isArray(p.buttons) || p.buttons.length !== MAX_BUTTONS) return false;
  if (!Array.isArray(p.touch_pads) || p.touch_pads.length !== MAX_TOUCH_PADS) return false;
  if (!Array.isArray(p.pots) || p.pots.length !== MAX_POTS) return false;
  if (!p.ldr || typeof p.ldr.cc !== "number") return false;
  if (!p.accel || typeof p.accel.enabled !== "boolean") return false;

  for (const b of p.buttons) {
    if (typeof b.note !== "number" || typeof b.velocity !== "number") return false;
    if (!Array.isArray(b.note_expr) || !Array.isArray(b.velocity_expr)) return false;
  }
  for (const t of p.touch_pads) {
    if (typeof t.note !== "number" || typeof t.velocity !== "number") return false;
    if (typeof t.threshold_pct !== "number") return false;
    if (!Array.isArray(t.note_expr) || !Array.isArray(t.velocity_expr)) return false;
  }
  for (const p2 of p.pots) {
    if (typeof p2.cc !== "number") return false;
  }
  return true;
}

// ── (Item counts are fixed; add/remove functionality removed) ──
