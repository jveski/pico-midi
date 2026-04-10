export class AppToolbar extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.className = "toolbar";
    this.id = "toolbar";
    this.innerHTML = '<button class="btn btn-primary" id="btnConnect">Connect</button>';
  }

  get btnConnect() { return this.querySelector("#btnConnect"); }

  set connected(v) {
    this.btnConnect.disabled = v;
    if (v) {
      this.btnConnect.classList.remove("btn-primary");
    } else {
      this.btnConnect.classList.add("btn-primary");
    }
  }

  set busy(v) {
    // Toolbar busy state (reserved for future use)
  }
}

customElements.define("app-toolbar", AppToolbar);
