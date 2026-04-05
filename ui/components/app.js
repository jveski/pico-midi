import {
  cobsDecode, buildRequest, parseResponse, readMonitorSnapshot,
  PostcardReader,
  REQ_VERSION, REQ_GET_CONFIG, REQ_PUT_CONFIG, REQ_SAVE, REQ_RESET, REQ_REBOOT,
  RESP_MONITOR, MAX_BUTTONS, MAX_TOUCH_PADS, MAX_POTS,
} from "./protocol.js";
import { sleep, num, clamp } from "./helpers.js";

// ── State ──

let port = null, reader = null, writer = null, keepReading = false;
let rxBuf = [], rxFrames = [];
let cmdLock = Promise.resolve();
let config = null;
let monitorTapTimer = null;

// ── DOM refs (set by init) ──

let statusBar, toolbar, configPanel, emptyState, serialLog, toastEl;

export function init(refs) {
  statusBar = refs.statusBar;
  toolbar = refs.toolbar;
  configPanel = refs.configPanel;
  emptyState = refs.emptyState;
  serialLog = refs.serialLog;
  toastEl = refs.toast;

  toolbar.btnConnect.addEventListener("click", async () => {
    if (port) await disconnect(); else await connect();
  });
  toolbar.btnRefresh.addEventListener("click", refreshConfig);
  toolbar.btnSave.addEventListener("click", saveConfig);
  toolbar.btnReset.addEventListener("click", resetConfig);
  toolbar.btnReboot.addEventListener("click", rebootDevice);

  configPanel.addEventListener("item-add", handleItemAdd);
  configPanel.addEventListener("item-remove", handleItemRemove);

  if (!("serial" in navigator)) {
    document.getElementById("unsupported").style.display = "block";
    toolbar.btnConnect.disabled = true;
  } else {
    navigator.serial.addEventListener("disconnect", (e) => {
      if (port && (e.target === port || e.port === port)) {
        disconnect();
        toast("Device disconnected", "info");
      }
    });
  }
}

// ── Toast ──

function toast(msg, type) {
  toastEl.show(msg, type);
}

// ── Log ──

function log(text, cls) {
  serialLog.append(text, cls);
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
    log("Connected", "resp");
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
      log("Connection error: " + e.message, "err");
      toast("Connection failed", "error");
    }
    await disconnect();
  }
}

async function disconnect() {
  keepReading = false;
  try { if (reader) { await reader.cancel(); reader.releaseLock(); } } catch {}
  try { if (writer) { writer.releaseLock(); } } catch {}
  try { if (port) { await port.close(); } } catch {}
  reader = null; writer = null; port = null;
  rxBuf = []; rxFrames = [];
  cmdLock = Promise.resolve();
  setConnected(false);
  log("Disconnected", "err");
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
      if (keepReading) log("Read error: " + e.message, "err");
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
  const reqNames = ["PING", "VERSION", "GET_CONFIG", "PUT_CONFIG", "SAVE", "RESET", "REBOOT"];
  log("> " + (reqNames[variantIndex] || "?"), "cmd");

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
          const desc = resp.type === "error" ? "ERR: " + resp.message :
                       resp.type === "version" ? resp.value :
                       resp.type === "config" ? "Config (" + resp.value.buttons.length + " btns)" :
                       resp.type.toUpperCase();
          log(desc, resp.type === "error" ? "err" : "resp");
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

// ── Config Operations ──

async function refreshConfig() {
  setToolbarBusy(true);
  try {
    const resp = await sendRequest(REQ_GET_CONFIG);
    if (resp.type === "config") {
      config = resp.value;
      renderConfig();
      toast("Config loaded", "success");
    } else if (resp.type === "error") {
      throw new Error(resp.message);
    } else {
      throw new Error("Unexpected response: " + resp.type);
    }
  } catch (e) {
    log("Refresh failed: " + e.message, "err");
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
  if (!config) return;
  const panel = configPanel;
  config.midi_channel = clamp(num(panel.midiChannel.value, 0), 0, 15);

  // Buttons
  config.buttons = panel.buttonList.readFromDOM().map(b => ({
    pin: clamp(b.pin, 0, 29),
    note: clamp(b.note, 0, 127),
    velocity: clamp(b.velocity, 1, 127),
  }));

  // Touch pads
  config.touch_pads = panel.touchList.readFromDOM().map(t => ({
    pin: clamp(t.pin, 0, 29),
    note: clamp(t.note, 0, 127),
    velocity: clamp(t.velocity, 1, 127),
  }));

  // Pots
  config.pots = panel.potList.readFromDOM().map(p => ({
    pin: clamp(p.pin, 0, 29),
    cc: clamp(p.cc, 0, 127),
  }));

  config.ldr_enabled = document.getElementById("ldrEnabled").checked;
  config.ldr = {
    pin: clamp(num(document.getElementById("ldrPin").value, 0), 0, 29),
    cc: clamp(num(document.getElementById("ldrCc").value, 0), 0, 127),
  };

  config.accel_enabled = document.getElementById("accelEnabled").checked;
  config.accel = {
    sda: clamp(num(document.getElementById("accelSda").value, 0), 0, 29),
    scl: clamp(num(document.getElementById("accelScl").value, 0), 0, 29),
    int_pin: clamp(num(document.getElementById("accelInt").value, 0), 0, 29),
    x_cc: clamp(num(document.getElementById("accelXCc").value, 0), 0, 127),
    y_cc: clamp(num(document.getElementById("accelYCc").value, 0), 0, 127),
    tap_note: clamp(num(document.getElementById("accelTapNote").value, 0), 0, 127),
    tap_vel: clamp(num(document.getElementById("accelTapVel").value, 1), 1, 127),
    dead_zone: clamp(num(document.getElementById("accelDeadZone").value, 0), 0, 255),
    smoothing: clamp(num(document.getElementById("accelSmoothing").value, 0), 0, 100),
  };
}

async function applyConfig() {
  try {
    readConfigFromUI();
    const resp = await sendRequest(REQ_PUT_CONFIG, config);
    if (resp.type === "ok") return true;
    throw new Error(resp.type === "error" ? resp.message : "Unexpected: " + resp.type);
  } catch (e) {
    log("Apply failed: " + e.message, "err");
    toast("Apply failed: " + e.message, "error");
    return false;
  }
}

async function saveConfig() {
  setToolbarBusy(true);
  try {
    if (!await applyConfig()) return;
    const resp = await sendRequest(REQ_SAVE);
    if (resp.type === "ok") toast("Saved to flash", "success");
    else toast("Save failed: " + (resp.message || resp.type), "error");
  } catch (e) {
    log("Save error: " + e.message, "err");
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
      await refreshConfig();
      toast("Defaults restored", "info");
    }
  } catch (e) {
    log("Reset error: " + e.message, "err");
    toast("Reset failed", "error");
  } finally {
    setToolbarBusy(false);
  }
}

async function rebootDevice() {
  if (!confirm("Reboot the device? Unsaved changes will be lost.")) return;
  setToolbarBusy(true);
  try {
    await sendRequest(REQ_REBOOT);
    toast("Device rebooting...", "info");
    await disconnect();
  } catch (e) {
    await disconnect();
    toast("Device rebooted", "info");
  }
}

// ── Item Add/Remove Handlers ──

function handleItemAdd(e) {
  if (!config) return;
  const list = e.target;
  const type = list.dataset.type;

  if (type === "button" && config.buttons.length < 8) {
    config.buttons.push({ pin: 0, note: 60, velocity: 100 });
    list.render(config.buttons);
  } else if (type === "touch" && config.touch_pads.length < 8) {
    config.touch_pads.push({ pin: 0, note: 72, velocity: 100 });
    list.render(config.touch_pads);
  } else if (type === "pot" && config.pots.length < 4) {
    config.pots.push({ pin: 26, cc: 0 });
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
