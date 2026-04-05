export class StatusBar extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.innerHTML =
      '<div class="status-dot" id="statusDot"></div>' +
      '<span id="statusText">Disconnected</span>';
    this.className = "status-bar";
  }

  set connected(v) {
    const dot = this.querySelector("#statusDot");
    const text = this.querySelector("#statusText");
    dot.className = v ? "status-dot connected" : "status-dot";
    if (!v) text.textContent = "Disconnected";
  }

  set text(v) {
    this.querySelector("#statusText").textContent = v;
  }
}

customElements.define("status-bar", StatusBar);
