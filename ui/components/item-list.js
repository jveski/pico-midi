import { noteName, num, pinLabel, BUTTON_PINS, TOUCH_PINS, POT_PINS } from "./helpers.js";
import { compileExpr, disassemble } from "./expr.js";

export class ItemList extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;

    this._type = this.dataset.type;         // "button" | "touch" | "pot"
    this._max = parseInt(this.dataset.max, 10);
    this._listId = this.dataset.listId;     // e.g. "buttonList"
    this._countId = this.dataset.countId;   // e.g. "btnCount"
    this._addId = this.dataset.addId;       // e.g. "addButton"
    this._addLabel = this.dataset.addLabel; // e.g. "+ Add Button"
    this._fields = this._type === "pot" ? ["cc"] : this._type === "touch" ? ["note", "velocity", "threshold_pct"] : ["note", "velocity"];

    this.innerHTML =
      `<div id="${this._listId}"></div>` +
      '<div class="list-actions">' +
        `<button class="btn btn-sm" id="${this._addId}">${this._addLabel}</button>` +
      '</div>';

    this.querySelector(`#${this._addId}`).addEventListener("click", () => {
      this.dispatchEvent(new CustomEvent("item-add", { bubbles: true }));
    });
  }

  render(items) {
    const container = this.querySelector(`#${this._listId}`);
    container.innerHTML = "";
    this._updateBadge(items.length);

    const isPot = this._type === "pot";
    const hasExpr = !isPot; // buttons and touch pads support expressions
    const monPrefix = this._type === "button" ? "monBtn" : this._type === "touch" ? "monTouch" : "";
    const pinMap = this._type === "button" ? BUTTON_PINS : this._type === "touch" ? TOUCH_PINS : POT_PINS;

    items.forEach((item, i) => {
      const row = document.createElement("div");
      row.className = "item-row";

      const pin = i < pinMap.length ? pinLabel(pinMap[i]) : "";
      const pinHtml = `<span class="pin-label">${pin}</span>`;

      if (isPot) {
        row.innerHTML =
          `<span class="index">#${i + 1}</span>` +
          pinHtml +
          `<div class="monitor-bar-track" style="max-width:80px"><div class="monitor-bar-fill" id="monPotBar${i}"></div></div>` +
          `<span class="monitor-value" id="monPotVal${i}" style="min-width:24px">0</span>` +
          `<label>CC</label><input type="number" min="0" max="127" value="${item.cc}" data-type="${this._type}" data-idx="${i}" data-field="cc">` +
          `<button class="btn-remove" data-type="${this._type}" data-idx="${i}">Remove</button>`;
      } else if (this._type === "touch") {
        row.innerHTML =
          `<span class="index">#${i + 1}</span>` +
          pinHtml +
          `<div class="monitor-indicator" id="${monPrefix}${i}"></div>` +
          `<label>Note</label><input type="number" min="0" max="127" value="${item.note}" data-type="${this._type}" data-idx="${i}" data-field="note">` +
          `<span class="note-hint">${noteName(item.note)}</span>` +
          `<label>Vel</label><input type="number" min="1" max="127" value="${item.velocity}" data-type="${this._type}" data-idx="${i}" data-field="velocity">` +
          `<label>Thr%</label><input type="number" min="1" max="255" value="${item.threshold_pct}" data-type="${this._type}" data-idx="${i}" data-field="threshold_pct">` +
          `<button class="btn-remove" data-type="${this._type}" data-idx="${i}">Remove</button>`;
      } else {
        row.innerHTML =
          `<span class="index">#${i + 1}</span>` +
          pinHtml +
          `<div class="monitor-indicator" id="${monPrefix}${i}"></div>` +
          `<label>Note</label><input type="number" min="0" max="127" value="${item.note}" data-type="${this._type}" data-idx="${i}" data-field="note">` +
          `<span class="note-hint">${noteName(item.note)}</span>` +
          `<label>Vel</label><input type="number" min="1" max="127" value="${item.velocity}" data-type="${this._type}" data-idx="${i}" data-field="velocity">` +
          `<button class="btn-remove" data-type="${this._type}" data-idx="${i}">Remove</button>`;
      }

      // Add expression fields for buttons and touch pads
      if (hasExpr) {
        const exprRow = document.createElement("div");
        exprRow.className = "expr-row";
        const noteExprSrc = item.note_expr_src || "";
        const velExprSrc = item.velocity_expr_src || "";
        exprRow.innerHTML =
          `<span class="expr-label">expr</span>` +
          `<label>Note</label><input type="text" class="expr-input" placeholder="e.g. pot0 + 24" value="${this._escAttr(noteExprSrc)}" data-type="${this._type}" data-idx="${i}" data-field="note_expr_src">` +
          `<span class="expr-error" data-idx="${i}" data-field="note_expr_err"></span>` +
          `<label>Vel</label><input type="text" class="expr-input" placeholder="e.g. pot1" value="${this._escAttr(velExprSrc)}" data-type="${this._type}" data-idx="${i}" data-field="velocity_expr_src">` +
          `<span class="expr-error" data-idx="${i}" data-field="velocity_expr_err"></span>`;
        row.appendChild(exprRow);
      }

      container.appendChild(row);
    });

    // Note hint live updates
    container.querySelectorAll('input[data-field="note"]').forEach(inp => {
      inp.addEventListener("input", () => {
        const hint = inp.parentElement.querySelector(".note-hint");
        if (hint) hint.textContent = noteName(parseInt(inp.value, 10) || 0);
      });
    });

    // Expression validation on input — also notify parent so config can
    // be applied to the device in realtime (without requiring a save).
    container.querySelectorAll(".expr-input").forEach(inp => {
      inp.addEventListener("input", () => {
        const field = inp.dataset.field;
        const errField = field.replace("_src", "_err");
        const errEl = inp.parentElement.querySelector(`[data-field="${errField}"][data-idx="${inp.dataset.idx}"]`);
        const { error } = compileExpr(inp.value);
        if (errEl) errEl.textContent = error || "";
        inp.classList.toggle("expr-invalid", !!error);
        if (!error) {
          this.dispatchEvent(new CustomEvent("expr-change", { bubbles: true }));
        }
      });
    });

    // Remove handlers
    container.querySelectorAll(".btn-remove").forEach(btn => {
      btn.addEventListener("click", () => {
        const idx = parseInt(btn.dataset.idx, 10);
        this.dispatchEvent(new CustomEvent("item-remove", { bubbles: true, detail: { idx } }));
      });
    });

    this.querySelector(`#${this._addId}`).disabled = items.length >= this._max || items.length >= pinMap.length;
  }

  _escAttr(s) {
    return (s || "").replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
  }

  syncFromDOM(items) {
    const container = this.querySelector(`#${this._listId}`);
    container.querySelectorAll(".item-row").forEach((row, i) => {
      if (i < items.length) {
        for (const f of this._fields)
          items[i][f] = num(row.querySelector(`[data-field="${f}"]`).value, items[i][f]);
        // Sync expression source text
        const noteExprInput = row.querySelector('[data-field="note_expr_src"]');
        if (noteExprInput) items[i].note_expr_src = noteExprInput.value;
        const velExprInput = row.querySelector('[data-field="velocity_expr_src"]');
        if (velExprInput) items[i].velocity_expr_src = velExprInput.value;
      }
    });
  }

  readFromDOM() {
    const items = [];
    const isPot = this._type === "pot";
    const isTouch = this._type === "touch";
    this.querySelector(`#${this._listId}`).querySelectorAll(".item-row").forEach(row => {
      if (isPot) {
        items.push({
          cc: num(row.querySelector('[data-field="cc"]').value, 0),
        });
      } else if (isTouch) {
        items.push({
          note: num(row.querySelector('[data-field="note"]').value, 0),
          velocity: num(row.querySelector('[data-field="velocity"]').value, 100),
          threshold_pct: num(row.querySelector('[data-field="threshold_pct"]').value, 33),
          note_expr_src: (row.querySelector('[data-field="note_expr_src"]') || {}).value || "",
          velocity_expr_src: (row.querySelector('[data-field="velocity_expr_src"]') || {}).value || "",
        });
      } else {
        items.push({
          note: num(row.querySelector('[data-field="note"]').value, 0),
          velocity: num(row.querySelector('[data-field="velocity"]').value, 100),
          note_expr_src: (row.querySelector('[data-field="note_expr_src"]') || {}).value || "",
          velocity_expr_src: (row.querySelector('[data-field="velocity_expr_src"]') || {}).value || "",
        });
      }
    });
    return items;
  }

  _updateBadge(count) {
    const badge = document.getElementById(this._countId);
    if (badge) badge.textContent = count;
  }
}

customElements.define("item-list", ItemList);
