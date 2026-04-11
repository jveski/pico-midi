import { BaseElement, classProperty } from "./helpers.js";

export class ConnectBanner extends BaseElement {
  init() {}

  get btnConnect() { return this.querySelector("#btnConnect"); }

  showUnsupported() {
    this.querySelector(".connect-banner-text").textContent =
      "Web Serial API is not available in this browser. Use Chrome or Edge 89+ on desktop.";
    this.querySelector(".connect-banner-text").classList.add("unsupported");
    this.btnConnect.disabled = true;
  }
}

classProperty(ConnectBanner, "visible", "visible");
customElements.define("connect-banner", ConnectBanner);
