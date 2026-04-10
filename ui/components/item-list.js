import { noteName, num, pinLabel, BUTTON_PINS, TOUCH_PINS, POT_PINS } from "./helpers.js";
import { compileExpr } from "./expr.js";

export class ItemList extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;

    this._type = this.dataset.type;         // "button" | "touch" | "pot"
    this._listId = this.dataset.listId;     // e.g. "buttonList"
    this._countId = this.dataset.countId;   // e.g. "btnCount"

    // Container div is defined in configurator.html.
  }

  render(items) {
    const container = this.querySelector(`#${this._listId}`);
    container.innerHTML = "";
    this._updateBadge(items.length);

    const isPot = this._type === "pot";
    const monPrefix = this._type === "button" ? "monBtn" : this._type === "touch" ? "monTouch" : "";
    const pinMap = this._type === "button" ? BUTTON_PINS : this._type === "touch" ? TOUCH_PINS : POT_PINS;

    const tplId = isPot ? "tpl-pot-row"
      : this._type === "touch" ? "tpl-touch-row"
      : "tpl-button-row";
    const tpl = document.getElementById(tplId);

    items.forEach((item, i) => {
      const row = tpl.content.firstElementChild.cloneNode(true);

      const gpioNum = i < pinMap.length ? pinMap[i] : null;
      const pinEl = row.querySelector(".pin-label");
      pinEl.textContent = pinLabel(gpioNum);
      if (gpioNum != null) {
        pinEl.classList.add("clickable");
        pinEl.dataset.gpio = gpioNum;
      }

      row.querySelector(".index").textContent = "#" + (i + 1);

      if (isPot) {
        row.querySelector(".monitor-bar-fill").id = "monPotBar" + i;
        row.querySelector(".monitor-value").id = "monPotVal" + i;
        const ccInput = row.querySelector('[data-field="cc"]');
        ccInput.value = item.cc;
        ccInput.dataset.type = this._type;
        ccInput.dataset.idx = i;
      } else {
        const noteExprSrc = item.note_expr_src || String(item.note);
        const velExprSrc = item.velocity_expr_src || String(item.velocity);

        row.querySelector(".monitor-indicator").id = monPrefix + i;

        const noteInput = row.querySelector('[data-field="note_expr_src"]');
        noteInput.value = noteExprSrc;
        noteInput.dataset.type = this._type;
        noteInput.dataset.idx = i;

        row.querySelector(".note-hint").dataset.idx = i;

        const noteErr = row.querySelector('[data-field="note_expr_err"]');
        noteErr.dataset.idx = i;

        const velInput = row.querySelector('[data-field="velocity_expr_src"]');
        velInput.value = velExprSrc;
        velInput.dataset.type = this._type;
        velInput.dataset.idx = i;

        const velErr = row.querySelector('[data-field="velocity_expr_err"]');
        velErr.dataset.idx = i;

        if (this._type === "touch") {
          const thrInput = row.querySelector('[data-field="threshold_pct"]');
          thrInput.value = item.threshold_pct;
          thrInput.dataset.type = this._type;
          thrInput.dataset.idx = i;
        }
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
