export class ToastNotification extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.id = "toast";
    this.className = "toast";
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
