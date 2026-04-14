import { BaseElement, num, noteName, toggleFieldsVisibility, analogPinOptions, refreshSelectPinConstraints, updateHint, readClamped } from "./helpers.js";

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

export class LdrSection extends BaseElement {
  init() {
    this.querySelector("#ldrEnabled").addEventListener("change", () => {
      this._updateVisibility();
      this._buildMonitor();
    });
  }

  render(config, usedAnalog = new Set()) {
    this.querySelector("#ldrEnabled").checked = config.ldr_enabled;
    this.querySelector("#ldrCc").value = config.ldr.cc;

    // Populate pin selector
    const pinSelect = this.querySelector("#ldrPin");
    if (pinSelect) {
      pinSelect.innerHTML = analogPinOptions(config.ldr.pin, usedAnalog);
    }

    this._updateVisibility();
    this._buildMonitor(config);
  }

  readFromDOM() {
    const pinSelect = this.querySelector("#ldrPin");
    return {
      ldr_enabled: this.querySelector("#ldrEnabled").checked,
      ldr: {
        pin: pinSelect ? num(pinSelect.value, 28) : 28,
        cc: readClamped(this, "ldrCc", 0, 0, 127),
      },
    };
  }

  _updateVisibility() {
    toggleFieldsVisibility(this, "ldrEnabled", "ldrFields");
  }

  _buildMonitor(config) {
    const container = this.querySelector("#ldrMonitor");
    container.innerHTML = "";
    if (!(config ? config.ldr_enabled : this.querySelector("#ldrEnabled").checked)) return;
    buildMonitorBars(container, [
      { label: "Value", barId: "monLdrBar", valId: "monLdrVal" },
    ]);
  }

  refreshPinOptions(usedAnalog) {
    const pinSelect = this.querySelector("#ldrPin");
    if (pinSelect) refreshSelectPinConstraints(pinSelect, usedAnalog);
  }
}

export class AccelSection extends BaseElement {
  init() {
    this.querySelector("#accelEnabled").addEventListener("change", () => {
      this._updateVisibility();
      this._buildMonitor();
    });

    this.querySelector("#accelTapNote").addEventListener("input", () => this._updateHints());
    this.querySelector("#accelDeadZone").addEventListener("input", () => this._updateHints());
    this.querySelector("#accelSmoothing").addEventListener("input", () => this._updateHints());
  }

  render(config) {
    this.querySelector("#accelEnabled").checked = config.accel.enabled;
    const chipSelect = this.querySelector("#accelChip");
    if (chipSelect) chipSelect.value = config.accel.chip ?? 0;
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
    const chipSelect = this.querySelector("#accelChip");
    return {
      enabled: this.querySelector("#accelEnabled").checked,
      chip: chipSelect ? num(chipSelect.value, 0) : 0,
      x_cc: readClamped(this, "accelXCc", 0, 0, 127),
      y_cc: readClamped(this, "accelYCc", 0, 0, 127),
      tap_note: readClamped(this, "accelTapNote", 0, 0, 127),
      tap_velocity: readClamped(this, "accelTapVel", 1, 1, 127),
      dead_zone_tenths: readClamped(this, "accelDeadZone", 0, 0, 255),
      smoothing_pct: readClamped(this, "accelSmoothing", 0, 0, 100),
    };
  }

  _updateVisibility() {
    toggleFieldsVisibility(this, "accelEnabled", "accelFields");
  }

  _updateHints() {
    updateHint(this, "accelTapNote", "tapNoteHint", noteName);
    updateHint(this, "accelDeadZone", "deadZoneHint", v => (v / 10).toFixed(1) + " m/s\u00B2");
    updateHint(this, "accelSmoothing", "smoothingHint", v => "\u03B1=" + (v / 100).toFixed(2));
  }

  _buildMonitor(config) {
    const container = this.querySelector("#accelMonitor");
    container.innerHTML = "";
    if (!(config ? config.accel.enabled : this.querySelector("#accelEnabled").checked)) return;

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
