export class ConnectBanner extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.className = "connect-banner visible";
    this.innerHTML =
      '<span class="connect-banner-text">No device connected</span>' +
      '<button class="btn btn-primary" id="btnConnect">Connect</button>';
  }

  get btnConnect() { return this.querySelector("#btnConnect"); }

  set visible(v) {
    this.classList.toggle("visible", !!v);
  }

  get visible() {
    return this.classList.contains("visible");
  }
}

customElements.define("connect-banner", ConnectBanner);
