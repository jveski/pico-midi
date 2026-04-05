export class SerialLog extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.className = "log-panel";
    this.innerHTML =
      '<div class="card-header collapsed" data-section="log">' +
        '<h2>Serial Log</h2>' +
        '<span class="chevron">&#9660;</span>' +
      '</div>' +
      '<div class="card-body collapsed" id="sectionLog">' +
        '<div class="log-content" id="logContent"></div>' +
      '</div>';

    const header = this.querySelector(".card-header");
    const body = this.querySelector(".card-body");
    header.addEventListener("click", () => {
      const collapsed = body.classList.toggle("collapsed");
      header.classList.toggle("collapsed", collapsed);
    });
  }

  append(text, cls) {
    const el = this.querySelector("#logContent");
    const span = document.createElement("span");
    span.className = cls || "";
    span.textContent = text + "\n";
    el.appendChild(span);
    el.scrollTop = el.scrollHeight;
  }
}

customElements.define("serial-log", SerialLog);
