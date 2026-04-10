import { noteName, pinLabel, LDR_PIN, ACCEL_SCL_PIN, ACCEL_SDA_PIN } from "./helpers.js";

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
    this.querySelector(".pin-label.clickable").addEventListener("click", () => {
      const modal = document.querySelector("pinout-modal");
      if (modal) modal.show(LDR_PIN);
    });
  }

  render(config) {
    this.querySelector("#ldrEnabled").checked = config.ldr_enabled;
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
    const tpl = document.getElementById("tpl-monitor-bar-row");
    const row = tpl.content.firstElementChild.cloneNode(true);
    row.querySelector(".monitor-label").textContent = "Value";
    row.querySelector(".monitor-bar-fill").id = "monLdrBar";
    row.querySelector(".monitor-value").id = "monLdrVal";
    container.appendChild(row);
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
    this.querySelectorAll(".pin-label.clickable").forEach(span => {
      span.addEventListener("click", () => {
        const gpio = parseInt(span.dataset.gpio, 10);
        const modal = document.querySelector("pinout-modal");
        if (modal && !isNaN(gpio)) modal.show(gpio);
      });
    });
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
    const enabled = config ? config.accel.enabled : this.querySelector("#accelEnabled").checked;
    if (!enabled) return;

    const barTpl = document.getElementById("tpl-monitor-bar-row");

    [["Tilt X", "monAccelXBar", "monAccelXVal", "64"],
     ["Tilt Y", "monAccelYBar", "monAccelYVal", "64"]].forEach(([label, barId, valId, def]) => {
      const row = barTpl.content.firstElementChild.cloneNode(true);
      row.querySelector(".monitor-label").textContent = label;
      row.querySelector(".monitor-bar-fill").id = barId;
      const valEl = row.querySelector(".monitor-value");
      valEl.id = valId;
      valEl.textContent = def;
      container.appendChild(row);
    });

    const tapTpl = document.getElementById("tpl-monitor-tap-row");
    const tapRow = tapTpl.content.firstElementChild.cloneNode(true);
    tapRow.querySelector(".monitor-tap").id = "monAccelTap";
    container.appendChild(tapRow);
  }
}

customElements.define("ldr-section", LdrSection);
customElements.define("accel-section", AccelSection);
