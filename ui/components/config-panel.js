import { BaseElement } from "./helpers.js";

export class ConfigPanel extends BaseElement {
  init() {
    this._wireProjectActions();
  }

  _wireProjectActions() {
    this.querySelector("#btnImport").addEventListener("click", () => {
      this.querySelector("#importFile").click();
    });

    this.querySelector("#importFile").addEventListener("change", (e) => {
      const file = e.target.files[0];
      if (file) {
        this.emit("project-import", { file });
      }
      e.target.value = "";
    });
  }

  get buttonList() { return this.querySelector('item-list[data-type="button"]'); }
  get touchList() { return this.querySelector('item-list[data-type="touch"]'); }
  get potList() { return this.querySelector('item-list[data-type="pot"]'); }
  get ldrSection() { return this.querySelector("ldr-section"); }
  get accelSection() { return this.querySelector("accel-section"); }
  get loopControl() { return this.querySelector("loop-control"); }
  get midiChannel() { return this.querySelector("midi-channel"); }
  get btnExport() { return this.querySelector("#btnExport"); }
  get btnImport() { return this.querySelector("#btnImport"); }
  get btnReset() { return this.querySelector("#btnReset"); }
  get pinoutGuide() { return this.querySelector("pinout-guide"); }

  set projectBusy(v) {
    this.btnExport.disabled = v;
    this.btnImport.disabled = v;
    this.btnReset.disabled = v;
  }

  set disabled(v) {
    const on = !!v;
    this.classList.toggle("disabled", on);
    // Disable interactive controls when no device is connected
    this.querySelectorAll("input, select, button.btn, .toggle").forEach(el => {
      el.inert = on;
    });
  }

  get disabled() {
    return this.classList.contains("disabled");
  }
}

customElements.define("config-panel", ConfigPanel);
