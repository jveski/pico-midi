import { BaseElement, classProperty } from "./helpers.js";

export class SaveBanner extends BaseElement {
  init() {}

  get btnSave() { return this.querySelector("#btnSave"); }

  set busy(v) {
    this.btnSave.disabled = !!v;
  }
}

classProperty(SaveBanner, "visible", "visible");
customElements.define("save-banner", SaveBanner);
