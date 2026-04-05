import { noteName } from "./helpers.js";

export class LdrSection extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.innerHTML =
      '<div class="toggle-row">' +
        '<label class="toggle">' +
          '<input type="checkbox" id="ldrEnabled">' +
          '<span class="slider"></span>' +
        '</label>' +
        '<label>Enabled</label>' +
      '</div>' +
      '<div id="ldrFields">' +
        '<div class="field">' +
          '<label>Pin (GPIO)</label>' +
          '<input type="number" id="ldrPin" min="0" max="29" value="28">' +
        '</div>' +
        '<div class="field">' +
          '<label>CC Number</label>' +
          '<input type="number" id="ldrCc" min="0" max="127" value="74">' +
        '</div>' +
        '<div id="ldrMonitor"></div>' +
      '</div>';

    this.querySelector("#ldrEnabled").addEventListener("change", () => {
      this._updateVisibility();
      this._buildMonitor();
    });
  }

  render(config) {
    this.querySelector("#ldrEnabled").checked = config.ldr_enabled;
    this.querySelector("#ldrPin").value = config.ldr.pin;
    this.querySelector("#ldrCc").value = config.ldr.cc;
    this._updateVisibility();
    this._buildMonitor(config);
  }

  _updateVisibility() {
    this.querySelector("#ldrFields").style.display =
      this.querySelector("#ldrEnabled").checked ? "" : "none";
  }

  _buildMonitor(config) {
    const container = this.querySelector("#ldrMonitor");
    container.innerHTML = "";
    const enabled = config ? config.ldr_enabled : this.querySelector("#ldrEnabled").checked;
    if (!enabled) return;
    const row = document.createElement("div");
    row.className = "monitor-row";
    row.innerHTML =
      '<span class="monitor-label">Value</span>' +
      '<div class="monitor-bar-track"><div class="monitor-bar-fill" id="monLdrBar"></div></div>' +
      '<span class="monitor-value" id="monLdrVal">0</span>';
    container.appendChild(row);
  }
}

export class AccelSection extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.innerHTML =
      '<div class="toggle-row">' +
        '<label class="toggle">' +
          '<input type="checkbox" id="accelEnabled">' +
          '<span class="slider"></span>' +
        '</label>' +
        '<label>Enabled</label>' +
      '</div>' +
      '<div id="accelFields">' +
        '<div class="field"><label>SDA Pin</label><input type="number" id="accelSda" min="0" max="29" value="0"></div>' +
        '<div class="field"><label>SCL Pin</label><input type="number" id="accelScl" min="0" max="29" value="1"></div>' +
        '<div class="field"><label>INT Pin</label><input type="number" id="accelInt" min="0" max="29" value="11"></div>' +
        '<div class="field"><label>X Axis CC</label><input type="number" id="accelXCc" min="0" max="127" value="1"></div>' +
        '<div class="field"><label>Y Axis CC</label><input type="number" id="accelYCc" min="0" max="127" value="2"></div>' +
        '<div class="field"><label>Tap Note</label><input type="number" id="accelTapNote" min="0" max="127" value="48"><span class="note-hint" id="tapNoteHint"></span></div>' +
        '<div class="field"><label>Tap Velocity</label><input type="number" id="accelTapVel" min="1" max="127" value="127"></div>' +
        '<div class="field"><label>Dead Zone</label><input type="number" id="accelDeadZone" min="0" max="255" value="13"><span class="note-hint" id="deadZoneHint"></span></div>' +
        '<div class="field"><label>Smoothing</label><input type="number" id="accelSmoothing" min="0" max="100" value="25"><span class="note-hint" id="smoothingHint"></span></div>' +
        '<div id="accelMonitor"></div>' +
      '</div>';

    this.querySelector("#accelEnabled").addEventListener("change", () => {
      this._updateVisibility();
      this._buildMonitor();
    });

    this.querySelector("#accelTapNote").addEventListener("input", () => this._updateHints());
    this.querySelector("#accelDeadZone").addEventListener("input", () => this._updateHints());
    this.querySelector("#accelSmoothing").addEventListener("input", () => this._updateHints());
  }

  render(config) {
    this.querySelector("#accelEnabled").checked = config.accel_enabled;
    this.querySelector("#accelSda").value = config.accel.sda;
    this.querySelector("#accelScl").value = config.accel.scl;
    this.querySelector("#accelInt").value = config.accel.int_pin;
    this.querySelector("#accelXCc").value = config.accel.x_cc;
    this.querySelector("#accelYCc").value = config.accel.y_cc;
    this.querySelector("#accelTapNote").value = config.accel.tap_note;
    this.querySelector("#accelTapVel").value = config.accel.tap_vel;
    this.querySelector("#accelDeadZone").value = config.accel.dead_zone;
    this.querySelector("#accelSmoothing").value = config.accel.smoothing;
    this._updateVisibility();
    this._updateHints();
    this._buildMonitor(config);
  }

  _updateVisibility() {
    this.querySelector("#accelFields").style.display =
      this.querySelector("#accelEnabled").checked ? "" : "none";
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
    const enabled = config ? config.accel_enabled : this.querySelector("#accelEnabled").checked;
    if (!enabled) return;

    [["Tilt X", "monAccelXBar", "monAccelXVal", "64"],
     ["Tilt Y", "monAccelYBar", "monAccelYVal", "64"]].forEach(([label, barId, valId, def]) => {
      const row = document.createElement("div");
      row.className = "monitor-row";
      row.innerHTML =
        `<span class="monitor-label">${label}</span>` +
        `<div class="monitor-bar-track"><div class="monitor-bar-fill" id="${barId}"></div></div>` +
        `<span class="monitor-value" id="${valId}">${def}</span>`;
      container.appendChild(row);
    });

    const tapRow = document.createElement("div");
    tapRow.className = "monitor-row";
    tapRow.innerHTML =
      '<span class="monitor-label">Tap</span>' +
      '<div class="monitor-tap" id="monAccelTap"></div>';
    container.appendChild(tapRow);
  }
}

customElements.define("ldr-section", LdrSection);
customElements.define("accel-section", AccelSection);
