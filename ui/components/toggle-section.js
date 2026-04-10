import { num, clamp, noteName, wirePinClicks, toggleFieldsVisibility } from "./helpers.js";

// ── Shared monitor-bar helper ──

function buildMonitorBars(container, bars) {
  const tpl = document.getElementById("tpl-monitor-bar-row");
  for (const { label, barId, valId, defaultVal } of bars) {
    const row = tpl.content.firstElementChild.cloneNode(true);
    row.querySelector(".monitor-label").textContent = label;
    row.querySelector(".monitor-bar-fill").id = barId;
    const valEl = row.querySelector(".monitor-value");
    valEl.id = valId;
    if (defaultVal != null) valEl.textContent = defaultVal;
    container.appendChild(row);
  }
}

export class LdrSection extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    // HTML structure is defined in configurator.html.

    this.querySelector("#ldrEnabled").addEventListener("change", () => {
      this._updateVisibility();
      this._buildMonitor();
    });

    // Pin label click → open pinout modal
    wirePinClicks(this);
  }

  render(config) {
    this.querySelector("#ldrEnabled").checked = config.ldr_enabled;
    this.querySelector("#ldrCc").value = config.ldr.cc;
    this._updateVisibility();
    this._buildMonitor(config);
  }

  readFromDOM() {
    return {
      ldr_enabled: this.querySelector("#ldrEnabled").checked,
      ldr: { cc: clamp(num(this.querySelector("#ldrCc").value, 0), 0, 127) },
    };
  }

  _updateVisibility() {
    toggleFieldsVisibility(this, "ldrEnabled", "ldrFields");
  }

  _buildMonitor(config) {
    const container = this.querySelector("#ldrMonitor");
    container.innerHTML = "";
    const enabled = config ? config.ldr_enabled : this.querySelector("#ldrEnabled").checked;
    if (!enabled) return;
    buildMonitorBars(container, [
      { label: "Value", barId: "monLdrBar", valId: "monLdrVal" },
    ]);
  }
}

export class AccelSection extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    // HTML structure is defined in configurator.html.

    this.querySelector("#accelEnabled").addEventListener("change", () => {
      this._updateVisibility();
      this._buildMonitor();
    });

    this.querySelector("#accelTapNote").addEventListener("input", () => this._updateHints());
    this.querySelector("#accelDeadZone").addEventListener("input", () => this._updateHints());
    this.querySelector("#accelSmoothing").addEventListener("input", () => this._updateHints());

    // Pin label click → open pinout modal
    wirePinClicks(this);
  }

  render(config) {
    this.querySelector("#accelEnabled").checked = config.accel.enabled;
    this.querySelector("#accelXCc").value = config.accel.x_cc;
    this.querySelector("#accelYCc").value = config.accel.y_cc;
    this.querySelector("#accelTapNote").value = config.accel.tap_note;
    this.querySelector("#accelTapVel").value = config.accel.tap_velocity;
    this.querySelector("#accelDeadZone").value = config.accel.dead_zone_tenths;
    this.querySelector("#accelSmoothing").value = config.accel.smoothing_pct;
    this._updateVisibility();
    this._updateHints();
    this._buildMonitor(config);
  }

  readFromDOM() {
    return {
      enabled: this.querySelector("#accelEnabled").checked,
      x_cc: clamp(num(this.querySelector("#accelXCc").value, 0), 0, 127),
      y_cc: clamp(num(this.querySelector("#accelYCc").value, 0), 0, 127),
      tap_note: clamp(num(this.querySelector("#accelTapNote").value, 0), 0, 127),
      tap_velocity: clamp(num(this.querySelector("#accelTapVel").value, 1), 1, 127),
      dead_zone_tenths: clamp(num(this.querySelector("#accelDeadZone").value, 0), 0, 255),
      smoothing_pct: clamp(num(this.querySelector("#accelSmoothing").value, 0), 0, 100),
    };
  }

  _updateVisibility() {
    toggleFieldsVisibility(this, "accelEnabled", "accelFields");
  }

  _updateHints() {
    const tapNote = parseInt(this.querySelector("#accelTapNote").value, 10);
    this.querySelector("#tapNoteHint").textContent = isNaN(tapNote) ? "" : noteName(tapNote);
    const dz = parseInt(this.querySelector("#accelDeadZone").value, 10);
    this.querySelector("#deadZoneHint").textContent = isNaN(dz) ? "" : (dz / 10).toFixed(1) + " m/s\u00B2";
    const sm = parseInt(this.querySelector("#accelSmoothing").value, 10);
    this.querySelector("#smoothingHint").textContent = isNaN(sm) ? "" : "\u03B1=" + (sm / 100).toFixed(2);
  }

  _buildMonitor(config) {
    const container = this.querySelector("#accelMonitor");
    container.innerHTML = "";
    const enabled = config ? config.accel.enabled : this.querySelector("#accelEnabled").checked;
    if (!enabled) return;

    buildMonitorBars(container, [
      { label: "Tilt X", barId: "monAccelXBar", valId: "monAccelXVal", defaultVal: "64" },
      { label: "Tilt Y", barId: "monAccelYBar", valId: "monAccelYVal", defaultVal: "64" },
    ]);

    const tapTpl = document.getElementById("tpl-monitor-tap-row");
    const tapRow = tapTpl.content.firstElementChild.cloneNode(true);
    tapRow.querySelector(".monitor-tap").id = "monAccelTap";
    container.appendChild(tapRow);
  }
}

customElements.define("ldr-section", LdrSection);
customElements.define("accel-section", AccelSection);
