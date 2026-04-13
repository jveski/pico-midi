import { BaseElement } from "./helpers.js";

export class ContextPanel extends BaseElement {
  init() {
    this._tabs = this.querySelectorAll(".context-tab");
    this._panes = this.querySelectorAll(".context-pane");

    this.querySelector(".context-panel-header").addEventListener("click", (e) => {
      const tab = e.target.closest(".context-tab");
      if (tab) this.switchTo(tab.dataset.tab);
    });
  }

  /** Switch to a named tab: "pinout", "wiring", or "expr" */
  switchTo(name) {
    this._tabs.forEach(t => t.classList.toggle("active", t.dataset.tab === name));
    this._panes.forEach(p => {
      const paneKey = p.id.replace("context", "").toLowerCase();
      p.classList.toggle("active", paneKey === name);
    });
  }

  get activeTab() {
    const t = this.querySelector(".context-tab.active");
    return t ? t.dataset.tab : "pinout";
  }

  get pinoutGuide() {
    return this.querySelector("pinout-guide");
  }
}

customElements.define("context-panel", ContextPanel);
