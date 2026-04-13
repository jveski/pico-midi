import {
  cobsDecode, buildRequest, parseResponse, readMonitorSnapshot, readLoopState,
  PostcardReader,
  REQ_VERSION, REQ_GET_CONFIG, REQ_PUT_CONFIG, REQ_SAVE, REQ_RESET,
  REQ_LOOP_RECORD, REQ_LOOP_STOP_RECORD, REQ_LOOP_TOGGLE_MUTE,
  REQ_LOOP_CLEAR, REQ_LOOP_STOP_ALL, REQ_LOOP_PLAY, REQ_LOOP_STOP,
  RESP_MONITOR, RESP_LOOP_STATE, MAX_DIGITAL_INPUTS, MAX_ANALOG_INPUTS,
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

let connectBanner, layout, configPanel, contextPanel, sectionNav, saveBanner, toastEl;
let contextBackdrop = null;

export function init(refs) {
  connectBanner = refs.connectBanner;
  layout = refs.layout;
  configPanel = refs.configPanel;
  contextPanel = refs.contextPanel;
  sectionNav = refs.sectionNav;
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

  // Looper button events — delegate clicks from the loop-control component
  configPanel.addEventListener("click", handleLoopClick);

  // Context panel auto-switching: expression inputs -> expr ref, pin selects -> pinout
  configPanel.addEventListener("focusin", (e) => {
    if (!contextPanel) return;
    if (e.target.classList.contains("expr-input")) {
      contextPanel.switchTo("expr");
    } else if (e.target.classList.contains("pin-select")) {
      contextPanel.switchTo("pinout");
    }
  });

  // Mobile drawer toggle for context panel
  const toggleBtn = document.getElementById("btnContextToggle");
  if (toggleBtn) {
    // Create backdrop element for mobile drawer
    contextBackdrop = document.createElement("div");
    contextBackdrop.className = "context-backdrop";
    document.body.appendChild(contextBackdrop);

    toggleBtn.addEventListener("click", toggleContextDrawer);
    contextBackdrop.addEventListener("click", closeContextDrawer);
  }

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

function toggleContextDrawer() {
  if (!contextPanel) return;
  const isOpen = contextPanel.classList.contains("drawer-open");
  if (isOpen) {
    closeContextDrawer();
  } else {
    contextPanel.classList.add("drawer-open");
    if (contextBackdrop) contextBackdrop.classList.add("visible");
  }
}

function closeContextDrawer() {
  if (contextPanel) contextPanel.classList.remove("drawer-open");
  if (contextBackdrop) contextBackdrop.classList.remove("visible");
}

function toast(msg, type) {
  toastEl.show(msg, type);
}

function setConnected(connected) {
  connectBanner.visible = !connected;
  layout.classList.toggle("has-connect-banner", !connected);
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

function defaultSynthConfig() {
  return {
    enabled: false, audio_pin: 14,
    osc1_waveform: 0, osc2_waveform: 0,
    osc2_detune_cents: 7, osc2_semitone: 0,
    osc_mix: 64,
    filter_cutoff: 80, filter_resonance: 40, filter_env_amount: 64,
    amp_attack_ms: 10, amp_decay_ms: 200, amp_sustain_pct: 70, amp_release_ms: 300,
    filter_attack_ms: 5, filter_decay_ms: 300, filter_sustain_pct: 30, filter_release_ms: 200,
    master_volume: 80,
    reverb_mix: 40, reverb_size: 80, reverb_damping: 50,
    comp_mix: 0, comp_peak_reduction: 40, comp_gain: 30, comp_mode: 0,
  };
}

function defaultLoopConfig() {
  return {
    enabled: false,
    num_layers: 4,
    bpm: 120,
    quantize: 2,
    bars: 4,
  };
}

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
    synth: defaultSynthConfig(),
    loop_cfg: defaultLoopConfig(),
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
  if (panel.synthControl) panel.synthControl.render(cfg);
  if (panel.loopControl) panel.loopControl.render(cfg);
  // Pinout guide is now in the context panel
  const pinout = contextPanel ? contextPanel.pinoutGuide : null;
  if (pinout) pinout.update(cfg);
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
    layout.classList.remove("has-connect-banner");
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
      if (variant === RESP_LOOP_STATE) {
        const state = readLoopState(r);
        applyLoopState(state);
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
      const v = r.varint();
      return v === RESP_MONITOR || v === RESP_LOOP_STATE;
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
        if (resp && resp.type !== "monitor" && resp.type !== "loop_state") {
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

function applyLoopState(state) {
  if (configPanel && configPanel.loopControl) {
    configPanel.loopControl.applyLoopState(state);
  }
}

function handleLoopClick(e) {
  const btn = e.target.closest("[data-action]");
  if (!btn || !btn.closest("loop-control")) return;
  const action = btn.dataset.action;
  const layer = btn.dataset.layer != null ? parseInt(btn.dataset.layer, 10) : null;

  switch (action) {
    case "play":
      sendRequest(REQ_LOOP_PLAY);
      break;
    case "stop":
      sendRequest(REQ_LOOP_STOP);
      break;
    case "stop-all":
      sendRequest(REQ_LOOP_STOP_ALL);
      break;
    case "record": {
      // Toggle: if already recording, stop; otherwise start
      const stateEl = document.getElementById(`loopLayerState${layer}`);
      const isRecording = stateEl && stateEl.textContent === "Rec";
      sendRequest(isRecording ? REQ_LOOP_STOP_RECORD : REQ_LOOP_RECORD, layer);
      break;
    }
    case "mute":
      sendRequest(REQ_LOOP_TOGGLE_MUTE, layer);
      break;
    case "clear":
      sendRequest(REQ_LOOP_CLEAR, layer);
      break;
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
  readConfigFromUI();
  if (!config) return;
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
      const extras = type === "touch" ? { threshold_pct: 33 } : {};
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

  // Synth config
  if (panel.synthControl) {
    config.synth = panel.synthControl.readFromDOM();
  }

  // Loop config
  if (panel.loopControl) {
    config.loop_cfg = panel.loopControl.readFromDOM();
  }

  return config;
}

async function applyConfig() {
  try {
    const cfg = readConfigFromUI();
    if (!cfg) return false;
    saveExprSources();
    const pinout = contextPanel ? contextPanel.pinoutGuide : null;
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
    _version: 2,
    midi_channel: cfg.midi_channel,
    buttons: cfg.buttons.map(b => normalizeInputItem(b)),
    touch_pads: cfg.touch_pads.map(t => normalizeInputItem(t, { threshold_pct: t.threshold_pct })),
    pots: cfg.pots.map(p => ({ pin: p.pin, cc: p.cc })),
    ldr_enabled: cfg.ldr_enabled,
    ldr: { pin: cfg.ldr.pin, cc: cfg.ldr.cc },
    accel: { ...cfg.accel },
    synth: { ...cfg.synth },
    loop_cfg: cfg.loop_cfg ? { ...cfg.loop_cfg } : defaultLoopConfig(),
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
        synth: project.synth ? { ...project.synth } : defaultSynthConfig(),
        loop_cfg: project.loop_cfg ? { ...project.loop_cfg } : defaultLoopConfig(),
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
