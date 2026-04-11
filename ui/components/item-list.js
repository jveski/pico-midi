import { BaseElement, num, noteHintText, digitalPinOptions, analogPinOptions } from "./helpers.js";
import { DIGITAL_PINS, ANALOG_PINS, MAX_DIGITAL_INPUTS, MAX_ANALOG_INPUTS } from "./protocol.js";
import { compileExpr } from "./expr.js";

export class ItemList extends BaseElement {
  init() {
    this._type = this.dataset.type;         // "button" | "touch" | "pot"
    this._listId = this.dataset.listId;     // e.g. "buttonList"
    this._countId = this.dataset.countId;   // e.g. "btnCount"

    // Wire up the "Add" button
    const addBtn = this.querySelector('[data-action="add"]');
    if (addBtn) {
      addBtn.addEventListener("click", () => {
        this.emit("item-add", { type: this._type });
      });
    }
  }

  /**
   * Render items into the list.
   * @param {Array} items - Array of item objects (with .pin fields).
   * @param {Set<number>} usedPins - All pins in use across the full config.
   */
  render(items, usedPins) {
    const container = this.querySelector(`#${this._listId}`);
    container.innerHTML = "";
    this._updateBadge(items.length);
    this._updateAddButton(items.length);

    const isPot = this._type === "pot";
    const isDigital = this._type === "button" || this._type === "touch";
    const monPrefix = this._type === "button" ? "monBtn" : this._type === "touch" ? "monTouch" : "";

    const tplId = isPot ? "tpl-pot-row"
      : this._type === "touch" ? "tpl-touch-row"
      : "tpl-button-row";
    const tpl = document.getElementById(tplId);

    items.forEach((item, i) => {
      const row = tpl.content.firstElementChild.cloneNode(true);

      // Pin selector
      const pinSelect = row.querySelector('[data-field="pin"]');
      if (pinSelect) {
        pinSelect.innerHTML = isDigital
          ? digitalPinOptions(item.pin, usedPins || new Set())
          : analogPinOptions(item.pin, usedPins || new Set());
        pinSelect.dataset.type = this._type;
        pinSelect.dataset.idx = i;
      }

      row.querySelector(".index").textContent = "#" + (i + 1);

      // Remove button
      const removeBtn = row.querySelector('[data-action="remove"]');
      if (removeBtn) {
        removeBtn.dataset.type = this._type;
        removeBtn.dataset.idx = i;
        removeBtn.addEventListener("click", () => {
          this.emit("item-remove", { type: this._type, index: i });
        });
      }

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
          if (hint) hint.textContent = error ? "" : noteHintText(inp.value);
        }

        if (!error) {
          this.emit("expr-change");
        }
      });
    });

    // Pin select change → notify parent
    container.querySelectorAll('.pin-select').forEach(sel => {
      sel.addEventListener("change", () => {
        this.emit("pin-change");
      });
    });
  }

  /** Show note name hints for note expression fields that contain plain numbers. */
  _updateNoteHints(container) {
    container.querySelectorAll('input[data-field="note_expr_src"]').forEach(inp => {
      const hint = container.querySelector(`.note-hint[data-idx="${inp.dataset.idx}"]`);
      if (hint) hint.textContent = noteHintText(inp.value);
    });
  }

  readFromDOM() {
    const items = [];
    const isPot = this._type === "pot";
    this.querySelector(`#${this._listId}`).querySelectorAll(".item-row").forEach(row => {
      const pinSelect = row.querySelector('[data-field="pin"]');
      const pin = pinSelect ? num(pinSelect.value, 0) : 0;

      if (isPot) {
        items.push({
          pin,
          cc: num(row.querySelector('[data-field="cc"]').value, 0),
        });
      } else {
        const item = {
          pin,
          note_expr_src: (row.querySelector('[data-field="note_expr_src"]') || {}).value || "",
          velocity_expr_src: (row.querySelector('[data-field="velocity_expr_src"]') || {}).value || "",
        };
        if (this._type === "touch") {
          item.threshold_pct = num(row.querySelector('[data-field="threshold_pct"]').value, 33);
        }
        items.push(item);
      }
    });
    return items;
  }

  _updateBadge(count) {
    const badge = document.getElementById(this._countId);
    if (badge) badge.textContent = count;
  }

  _updateAddButton(count) {
    const addBtn = this.querySelector('[data-action="add"]');
    if (!addBtn) return;
    const isPot = this._type === "pot";
    const max = isPot ? MAX_ANALOG_INPUTS : MAX_DIGITAL_INPUTS;
    addBtn.style.display = count >= max ? "none" : "";
  }

  /**
   * Refresh the disabled state of all pin <select> options without
   * rebuilding the DOM (preserves focus and selection).
   * @param {Set<number>} usedPins - All pins currently in use across the config.
   */
  refreshPinOptions(usedPins) {
    const isDigital = this._type === "button" || this._type === "touch";
    const validPins = isDigital ? DIGITAL_PINS : ANALOG_PINS;
    this.querySelectorAll('.pin-select').forEach(sel => {
      const currentPin = num(sel.value, 0);
      for (const opt of sel.options) {
        const p = num(opt.value, -1);
        if (!validPins.includes(p)) continue;
        opt.disabled = usedPins.has(p) && p !== currentPin;
      }
    });
  }
}

customElements.define("item-list", ItemList);
