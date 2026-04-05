export class AppToolbar extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.className = "toolbar";
    this.id = "toolbar";
    this.innerHTML =
      '<button class="btn btn-primary" id="btnConnect">Connect</button>' +
      '<button class="btn" id="btnRefresh" disabled>Refresh</button>' +
      '<button class="btn" id="btnSave" disabled>Save to Flash</button>' +
      '<button class="btn" id="btnReset" disabled>Reset Defaults</button>' +
      '<button class="btn btn-danger" id="btnReboot" disabled>Reboot</button>';
  }

  get btnConnect() { return this.querySelector("#btnConnect"); }
  get btnRefresh() { return this.querySelector("#btnRefresh"); }
  get btnSave() { return this.querySelector("#btnSave"); }
  get btnReset() { return this.querySelector("#btnReset"); }
  get btnReboot() { return this.querySelector("#btnReboot"); }

  set connected(v) {
    const btn = this.btnConnect;
    btn.textContent = v ? "Disconnect" : "Connect";
    btn.classList.toggle("btn-primary", !v);
    btn.classList.toggle("btn-danger", v);
    this.btnRefresh.disabled = !v;
    this.btnSave.disabled = !v;
    this.btnReset.disabled = !v;
    this.btnReboot.disabled = !v;
  }

  set busy(v) {
    this.btnRefresh.disabled = v;
    this.btnSave.disabled = v;
    this.btnReset.disabled = v;
    this.btnReboot.disabled = v;
  }
}

customElements.define("app-toolbar", AppToolbar);
