export class SaveBanner extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    // HTML structure is defined in configurator.html.
  }

  get btnSave() { return this.querySelector("#btnSave"); }

  set visible(v) {
    this.classList.toggle("visible", !!v);
  }

  get visible() {
    return this.classList.contains("visible");
  }

  set busy(v) {
    this.btnSave.disabled = !!v;
  }
}

customElements.define("save-banner", SaveBanner);
