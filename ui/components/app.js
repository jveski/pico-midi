import {
  cobsDecode, buildRequest, parseResponse, readMonitorSnapshot,
  PostcardReader,
  REQ_VERSION, REQ_GET_CONFIG, REQ_PUT_CONFIG, REQ_SAVE, REQ_RESET,
  RESP_MONITOR, MAX_BUTTONS, MAX_TOUCH_PADS, MAX_POTS,
} from "./protocol.js";
import { sleep, num, clamp, BUTTON_PINS, TOUCH_PINS, POT_PINS } from "./helpers.js";
import { compileExpr, disassemble } from "./expr.js";

// ── State ──

let port = null, reader = null, writer = null, keepReading = false;
let rxBuf = [], rxFrames = [];
let cmdLock = Promise.resolve();
let config = null;
let monitorTapTimer = null;
let exprApplyTimer = null;

// Expression source text is stored alongside config but not serialized to
// the device — only the compiled bytecode is sent. We keep the source
// in the config objects as `note_expr_src` / `velocity_expr_src` strings
// and persist them to localStorage so they survive page reloads.

// ── DOM refs (set by init) ──

let statusBar, toolbar, configPanel, emptyState, toastEl;

export function init(refs) {
  statusBar = refs.statusBar;
  toolbar = refs.toolbar;
  configPanel = refs.configPanel;
  emptyState = refs.emptyState;
  toastEl = refs.toast;

  toolbar.btnConnect.addEventListener("click", connect);
  toolbar.btnRefresh.addEventListener("click", refreshConfig);
  toolbar.btnSave.addEventListener("click", saveConfig);
  toolbar.btnReset.addEventListener("click", resetConfig);

  configPanel.addEventListener("item-add", handleItemAdd);
  configPanel.addEventListener("item-remove", handleItemRemove);
  configPanel.addEventListener("expr-change", debouncedApplyConfig);

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
}

function setToolbarBusy(busy) {
  toolbar.busy = busy;
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

function readConfigFromUI() {
  if (!config) return null;
  const panel = configPanel;
  config.midi_channel = clamp(num(panel.midiChannel.value, 0), 0, 15);

  // Buttons — compile expressions to bytecode
  config.buttons = panel.buttonList.readFromDOM().map(b => {
    const noteResult = compileExpr(b.note_expr_src);
    const velResult = compileExpr(b.velocity_expr_src);
    if (noteResult.error) return { error: `Button note expr: ${noteResult.error}` };
    if (velResult.error) return { error: `Button velocity expr: ${velResult.error}` };
    return {
      note: clamp(b.note, 0, 127),
      velocity: clamp(b.velocity, 1, 127),
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

  // Touch pads — compile expressions to bytecode
  config.touch_pads = panel.touchList.readFromDOM().map(t => {
    const noteResult = compileExpr(t.note_expr_src);
    const velResult = compileExpr(t.velocity_expr_src);
    if (noteResult.error) return { error: `Touch note expr: ${noteResult.error}` };
    if (velResult.error) return { error: `Touch velocity expr: ${velResult.error}` };
    return {
      note: clamp(t.note, 0, 127),
      velocity: clamp(t.velocity, 1, 127),
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
    if (resp.type === "ok") toast("Saved to flash", "success");
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
      toast("Defaults restored", "info");
    }
  } catch (e) {
    toast("Reset failed", "error");
  } finally {
    setToolbarBusy(false);
  }
}

// ── Item Add/Remove Handlers ──

function handleItemAdd(e) {
  if (!config) return;
  const list = e.target;
  const type = list.dataset.type;

  if (type === "button" && config.buttons.length < BUTTON_PINS.length) {
    config.buttons.push({ note: 60, velocity: 100, note_expr: [], velocity_expr: [], note_expr_src: "", velocity_expr_src: "" });
    list.render(config.buttons);
  } else if (type === "touch" && config.touch_pads.length < TOUCH_PINS.length) {
    config.touch_pads.push({ note: 72, velocity: 100, threshold_pct: 33, note_expr: [], velocity_expr: [], note_expr_src: "", velocity_expr_src: "" });
    list.render(config.touch_pads);
  } else if (type === "pot" && config.pots.length < POT_PINS.length) {
    config.pots.push({ cc: 0 });
    list.render(config.pots);
  }
}

function handleItemRemove(e) {
  if (!config) return;
  const list = e.target;
  const type = list.dataset.type;
  const idx = e.detail.idx;

  const items = type === "button" ? config.buttons :
                type === "touch" ? config.touch_pads :
                config.pots;

  // Sync DOM values before removing
  list.syncFromDOM(items);
  items.splice(idx, 1);
  list.render(items);
}
