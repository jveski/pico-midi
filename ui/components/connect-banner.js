import { BaseElement, classProperty } from "./helpers.js";

export class ConnectBanner extends BaseElement {
  init() {}

  get btnConnect() { return this.querySelector("#btnConnect"); }

  showUnsupported() {
    const el = this.querySelector(".connect-banner-text");
    el.textContent = "Web Serial API is not available in this browser. Use Chrome or Edge 89+ on desktop.";
    el.classList.add("unsupported");
    this.btnConnect.disabled = true;
  }
}

classProperty(ConnectBanner, "visible", "visible");
customElements.define("connect-banner", ConnectBanner);
