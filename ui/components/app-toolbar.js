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
      '<button class="btn" id="btnReset" disabled>Reset Defaults</button>';
  }

  get btnConnect() { return this.querySelector("#btnConnect"); }
  get btnRefresh() { return this.querySelector("#btnRefresh"); }
  get btnSave() { return this.querySelector("#btnSave"); }
  get btnReset() { return this.querySelector("#btnReset"); }

  set connected(v) {
    this.btnRefresh.disabled = !v;
    this.btnSave.disabled = !v;
    this.btnReset.disabled = !v;
    this.btnConnect.disabled = v;
    if (v) {
      this.btnConnect.classList.remove("btn-primary");
    } else {
      this.btnConnect.classList.add("btn-primary");
    }
  }

  set busy(v) {
    this.btnRefresh.disabled = v;
    this.btnSave.disabled = v;
    this.btnReset.disabled = v;
  }
}

customElements.define("app-toolbar", AppToolbar);
