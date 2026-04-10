export class ProjectSection extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.id = "projectSection";
    this.style.display = "none";

    this.innerHTML =
      '<div class="project-section">' +
        '<h2>Project</h2>' +
        '<div class="project-actions">' +
          '<button class="btn" id="btnExport">Export Project</button>' +
          '<button class="btn" id="btnImport">Import Project</button>' +
          '<button class="btn" id="btnReset">Reset Defaults</button>' +
        '</div>' +
        '<input type="file" id="importFile" accept=".json" style="display:none">' +
      '</div>';

    this.querySelector("#btnImport").addEventListener("click", () => {
      this.querySelector("#importFile").click();
    });

    this.querySelector("#importFile").addEventListener("change", (e) => {
      const file = e.target.files[0];
      if (file) {
        this.dispatchEvent(new CustomEvent("project-import", { detail: { file }, bubbles: true }));
      }
      // Reset so the same file can be selected again
      e.target.value = "";
    });
  }

  get btnExport() { return this.querySelector("#btnExport"); }
  get btnImport() { return this.querySelector("#btnImport"); }
  get btnReset() { return this.querySelector("#btnReset"); }

  set busy(v) {
    this.btnExport.disabled = v;
    this.btnImport.disabled = v;
    this.btnReset.disabled = v;
  }
}

customElements.define("project-section", ProjectSection);
