import { noteName, num, pinLabel, BUTTON_PINS, TOUCH_PINS, POT_PINS } from "./helpers.js";
import { compileExpr } from "./expr.js";

export class ItemList extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;

    this._type = this.dataset.type;         // "button" | "touch" | "pot"
    this._listId = this.dataset.listId;     // e.g. "buttonList"
    this._countId = this.dataset.countId;   // e.g. "btnCount"

    this.innerHTML = `<div id="${this._listId}"></div>`;
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

      const gpioNum = i < pinMap.length ? pinMap[i] : null;
      const pin = pinLabel(gpioNum);
      const pinHtml = gpioNum != null
        ? `<span class="pin-label clickable" data-gpio="${gpioNum}">${pin}</span>`
        : `<span class="pin-label">${pin}</span>`;

      if (isPot) {
        row.innerHTML =
          `<span class="index">#${i + 1}</span>` +
          pinHtml +
          `<div class="monitor-bar-track" style="max-width:80px"><div class="monitor-bar-fill" id="monPotBar${i}"></div></div>` +
          `<span class="monitor-value" id="monPotVal${i}" style="min-width:24px">0</span>` +
          `<label>CC</label><input type="number" min="0" max="127" value="${item.cc}" data-type="${this._type}" data-idx="${i}" data-field="cc">`;
      } else if (this._type === "touch") {
        // Touch pads: expression inputs for note & velocity, plus numeric threshold
        const noteExprSrc = item.note_expr_src || String(item.note);
        const velExprSrc = item.velocity_expr_src || String(item.velocity);
        row.innerHTML =
          `<span class="index">#${i + 1}</span>` +
          pinHtml +
          `<div class="monitor-indicator" id="${monPrefix}${i}"></div>` +
          `<label>Note</label><input type="text" class="expr-input" placeholder="e.g. pot0 + 24" value="${this._escAttr(noteExprSrc)}" data-type="${this._type}" data-idx="${i}" data-field="note_expr_src">` +
          `<span class="note-hint" data-idx="${i}"></span>` +
          `<span class="expr-error" data-idx="${i}" data-field="note_expr_err"></span>` +
          `<label>Vel</label><input type="text" class="expr-input" placeholder="e.g. pot1" value="${this._escAttr(velExprSrc)}" data-type="${this._type}" data-idx="${i}" data-field="velocity_expr_src">` +
          `<span class="expr-error" data-idx="${i}" data-field="velocity_expr_err"></span>` +
          `<label>Thr%</label><input type="number" min="1" max="255" value="${item.threshold_pct}" data-type="${this._type}" data-idx="${i}" data-field="threshold_pct">`;
      } else {
        // Buttons: expression inputs for note & velocity
        const noteExprSrc = item.note_expr_src || String(item.note);
        const velExprSrc = item.velocity_expr_src || String(item.velocity);
        row.innerHTML =
          `<span class="index">#${i + 1}</span>` +
          pinHtml +
          `<div class="monitor-indicator" id="${monPrefix}${i}"></div>` +
          `<label>Note</label><input type="text" class="expr-input" placeholder="e.g. pot0 + 24" value="${this._escAttr(noteExprSrc)}" data-type="${this._type}" data-idx="${i}" data-field="note_expr_src">` +
          `<span class="note-hint" data-idx="${i}"></span>` +
          `<span class="expr-error" data-idx="${i}" data-field="note_expr_err"></span>` +
          `<label>Vel</label><input type="text" class="expr-input" placeholder="e.g. pot1" value="${this._escAttr(velExprSrc)}" data-type="${this._type}" data-idx="${i}" data-field="velocity_expr_src">` +
          `<span class="expr-error" data-idx="${i}" data-field="velocity_expr_err"></span>`;
      }

      container.appendChild(row);
    });

    // Note hint updates — show note name when value is a plain number
    this._updateNoteHints(container);

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

        // Update note hint if this is a note expression field
        if (field === "note_expr_src") {
          const hint = inp.parentElement.querySelector(`.note-hint[data-idx="${inp.dataset.idx}"]`);
          if (hint) {
            const v = parseInt(inp.value, 10);
            hint.textContent = (!error && String(v) === inp.value.trim()) ? noteName(v) : "";
          }
        }

        if (!error) {
          this.dispatchEvent(new CustomEvent("expr-change", { bubbles: true }));
        }
      });
    });

    // Pin label click → open pinout modal
    container.querySelectorAll(".pin-label.clickable").forEach(span => {
      span.addEventListener("click", () => {
        const gpio = parseInt(span.dataset.gpio, 10);
        const modal = document.querySelector("pinout-modal");
        if (modal && !isNaN(gpio)) modal.show(gpio);
      });
    });
  }

  _escAttr(s) {
    return (s || "").replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
  }

  /** Show note name hints for note expression fields that contain plain numbers. */
  _updateNoteHints(container) {
    container.querySelectorAll('input[data-field="note_expr_src"]').forEach(inp => {
      const hint = container.querySelector(`.note-hint[data-idx="${inp.dataset.idx}"]`);
      if (hint) {
        const v = parseInt(inp.value, 10);
        hint.textContent = (String(v) === inp.value.trim()) ? noteName(v) : "";
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
          threshold_pct: num(row.querySelector('[data-field="threshold_pct"]').value, 33),
          note_expr_src: (row.querySelector('[data-field="note_expr_src"]') || {}).value || "",
          velocity_expr_src: (row.querySelector('[data-field="velocity_expr_src"]') || {}).value || "",
        });
      } else {
        items.push({
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
