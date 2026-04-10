export class SaveBanner extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.className = "save-banner";
    this.innerHTML =
      '<span class="save-banner-text">Unsaved changes</span>' +
      '<button class="btn btn-primary" id="btnSave">Save to Flash</button>';
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
