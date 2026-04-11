import { BaseElement } from "./helpers.js";

export class ToastNotification extends BaseElement {
  init() {
    this._timer = null;
  }

  show(msg, type) {
    this.textContent = msg;
    this.className = "toast " + type + " visible";
    clearTimeout(this._timer);
    this._timer = setTimeout(() => this.classList.remove("visible"), 2500);
  }
}

customElements.define("toast-notification", ToastNotification);
