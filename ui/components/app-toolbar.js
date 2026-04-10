export class AppToolbar extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.className = "toolbar";
    this.id = "toolbar";
    this.innerHTML =
      '<button class="btn btn-primary" id="btnConnect">Connect</button>' +
      '<button class="btn" id="btnSave" disabled>Save to Flash</button>';
  }

  get btnConnect() { return this.querySelector("#btnConnect"); }
  get btnSave() { return this.querySelector("#btnSave"); }

  set connected(v) {
    this.btnSave.disabled = !v;
    this.btnConnect.disabled = v;
    if (v) {
      this.btnConnect.classList.remove("btn-primary");
    } else {
      this.btnConnect.classList.add("btn-primary");
    }
  }

  set busy(v) {
    this.btnSave.disabled = v;
  }
}

customElements.define("app-toolbar", AppToolbar);
