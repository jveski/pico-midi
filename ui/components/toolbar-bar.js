import { BaseElement } from "./helpers.js";

export class ToolbarBar extends BaseElement {
  init() {
    this._dirty = false;
  }

  get btnConnect() { return this.querySelector("#btnConnect"); }
  get btnSave() { return this.querySelector("#btnSave"); }
  get btnPinout() { return this.querySelector("#btnPinout"); }
  get statusEl() { return this.querySelector("#toolbarStatus"); }

  showUnsupported() {
    this.statusEl.textContent = "Web Serial not supported";
    this.statusEl.classList.add("unsupported");
    this.btnConnect.disabled = true;
  }

  set saveVisible(v) {
    this._dirty = !!v;
    this.btnSave.disabled = !v;
    this.btnSave.classList.toggle("btn-save-active", !!v);
  }

  set saveBusy(v) {
    if (v) {
      this.btnSave.disabled = true;
    } else {
      // Restore disabled state based on whether there are pending changes
      this.btnSave.disabled = !this._dirty;
    }
  }

  set connected(v) {
    this.btnConnect.classList.toggle("btn-connect-active", !v);
    this.btnConnect.textContent = v ? "Disconnect" : "Connect";
  }
}

customElements.define("toolbar-bar", ToolbarBar);
