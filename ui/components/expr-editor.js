// Expression editor modal: visual keyboard + function/variable chips.
// Opens for a specific input field, provides a piano keyboard for note
// selection and clickable chips for building expressions.

import { BaseElement, NOTE_NAMES, noteName } from "./helpers.js";
import { compileExpr } from "./expr.js";

const KEYBOARD_START = 36; // C2
const KEYBOARD_END = 96;   // C7

// Which semitone offsets are black keys
const BLACK_OFFSETS = new Set([1, 3, 6, 8, 10]);

export class ExprEditor extends BaseElement {
  init() {
    this._targetInput = null;
    this._isNoteField = false;
    this._onApply = null;

    this.innerHTML = `
      <div class="modal-overlay ee-overlay">
        <div class="modal-dialog ee-dialog">
          <div class="modal-header">
            <h2 class="ee-title">Edit Expression</h2>
            <button class="btn btn-sm modal-close" title="Close">&times;</button>
          </div>
          <div class="modal-body ee-body">
            <div class="ee-input-row">
              <input type="text" class="ee-input" placeholder="Type an expression or click below" spellcheck="false" autocomplete="off">
              <span class="ee-hint"></span>
              <span class="ee-error"></span>
            </div>
            <div class="ee-keyboard-section">
              <h3>Select Note</h3>
              <div class="ee-keyboard-scroll">
                <div class="ee-keyboard"></div>
              </div>
            </div>
            <div class="ee-chips-section">
              <h3>Variables</h3>
              <div class="ee-chip-group" data-group="vars"></div>
              <h3>Functions</h3>
              <div class="ee-chip-group" data-group="funcs"></div>
              <h3>Scale Modes</h3>
              <div class="ee-chip-group" data-group="scales"></div>
              <h3>Operators</h3>
              <div class="ee-chip-group" data-group="ops"></div>
              <h3>Conditionals</h3>
              <div class="ee-chip-group" data-group="cond"></div>
            </div>
            <div class="ee-actions">
              <button class="btn btn-primary ee-apply">Apply</button>
              <button class="btn ee-cancel">Cancel</button>
            </div>
          </div>
        </div>
      </div>`;

    this._overlay = this.querySelector(".ee-overlay");
    this._input = this.querySelector(".ee-input");
    this._hint = this.querySelector(".ee-hint");
    this._error = this.querySelector(".ee-error");
    this._title = this.querySelector(".ee-title");
    this._keyboardSection = this.querySelector(".ee-keyboard-section");
    this._keyboard = this.querySelector(".ee-keyboard");

    // Close handlers
    this.querySelector(".modal-close").addEventListener("click", () => this.close());
    this.querySelector(".ee-cancel").addEventListener("click", () => this.close());
    this._overlay.addEventListener("click", (e) => {
      if (e.target === this._overlay) this.close();
    });

    // Apply
    this.querySelector(".ee-apply").addEventListener("click", () => this._apply());

    // Live validation
    this._input.addEventListener("input", () => this._validate());

    // Enter key applies
    this._input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") { e.preventDefault(); this._apply(); }
    });

    // Escape closes (with stopImmediatePropagation to prevent ref-modal handler)
    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && this.isOpen) {
        e.stopImmediatePropagation();
        this.close();
      }
    });

    // Build keyboard
    this._buildKeyboard();

    // Build chips
    this._buildChips();
  }

  _buildKeyboard() {
    // Build a piano keyboard from KEYBOARD_START to KEYBOARD_END
    const kb = this._keyboard;
    // Calculate total white keys for width
    let whiteCount = 0;
    for (let n = KEYBOARD_START; n <= KEYBOARD_END; n++) {
      if (!BLACK_OFFSETS.has(n % 12)) whiteCount++;
    }

    const WHITE_W = 32;
    const BLACK_W = 20;
    const KEY_H = 100;
    const BLACK_H = 60;

    kb.style.width = (whiteCount * WHITE_W) + "px";
    kb.style.height = KEY_H + "px";
    kb.style.position = "relative";

    let whiteX = 0;
    const keys = [];

    // First pass: create white keys
    for (let n = KEYBOARD_START; n <= KEYBOARD_END; n++) {
      const isBlack = BLACK_OFFSETS.has(n % 12);
      if (isBlack) continue;
      const key = document.createElement("div");
      key.className = "ee-key ee-key-white";
      key.dataset.note = n;
      key.style.left = whiteX + "px";
      key.style.width = WHITE_W + "px";
      key.style.height = KEY_H + "px";

      // Label C notes
      if (n % 12 === 0) {
        const label = document.createElement("span");
        label.className = "ee-key-label";
        label.textContent = "C" + (Math.floor(n / 12) - 1);
        key.appendChild(label);
      }

      kb.appendChild(key);
      keys.push({ note: n, x: whiteX, w: WHITE_W });
      whiteX += WHITE_W;
    }

    // Second pass: create black keys (positioned between white keys)
    for (let n = KEYBOARD_START; n <= KEYBOARD_END; n++) {
      const isBlack = BLACK_OFFSETS.has(n % 12);
      if (!isBlack) continue;

      // Find the white key just before this black key
      const prevWhite = keys.find(k => k.note === n - 1);
      if (!prevWhite) continue;

      const key = document.createElement("div");
      key.className = "ee-key ee-key-black";
      key.dataset.note = n;
      key.style.left = (prevWhite.x + prevWhite.w - BLACK_W / 2) + "px";
      key.style.width = BLACK_W + "px";
      key.style.height = BLACK_H + "px";
      kb.appendChild(key);
    }

    // Click handler
    kb.addEventListener("click", (e) => {
      const key = e.target.closest(".ee-key");
      if (!key) return;
      const note = parseInt(key.dataset.note, 10);
      this._insertText(String(note));
    });
  }

  _buildChips() {
    const vars = [
      ["pot0", "Potentiometer 0"],
      ["pot1", "Potentiometer 1"],
      ["pot2", "Potentiometer 2"],
      ["ldr", "Light sensor"],
      ["accel_x", "Accelerometer X"],
      ["accel_y", "Accelerometer Y"],
    ];

    const funcs = [
      ["min(a, b)", "min(", "Minimum of two values"],
      ["max(a, b)", "max(", "Maximum of two values"],
      ["clamp(val, lo, hi)", "clamp(", "Clamp to range"],
      ["lerp(a, b, t)", "lerp(", "Interpolate a→b"],
      ["scale(root, pos)", "scale(", "Quantize to scale"],
    ];

    const scales = [
      ["lydian", "Bright, dreamy"],
      ["major", "Happy, standard"],
      ["mixolydian", "Bluesy"],
      ["dorian", "Jazzy minor"],
      ["minor", "Natural minor"],
      ["phrygian", "Dark, Spanish"],
      ["locrian", "Dissonant"],
    ];

    const ops = [
      ["+", "Add (saturating)"],
      ["-", "Subtract (saturating)"],
      ["*", "Multiply (saturating)"],
      ["/", "Integer divide"],
    ];

    const cond = [
      ["a > b ? x : y", " > 0 ? 0 : 0", "If greater than"],
    ];

    const makeChip = (label, insert, desc) => {
      const chip = document.createElement("button");
      chip.className = "btn btn-sm ee-chip";
      chip.textContent = label;
      chip.title = desc;
      chip.addEventListener("click", () => this._insertText(insert));
      return chip;
    };

    const varsGroup = this.querySelector('[data-group="vars"]');
    for (const [name, desc] of vars) {
      varsGroup.appendChild(makeChip(name, name, desc));
    }

    const funcsGroup = this.querySelector('[data-group="funcs"]');
    for (const [label, insert, desc] of funcs) {
      funcsGroup.appendChild(makeChip(label, insert, desc));
    }

    const scalesGroup = this.querySelector('[data-group="scales"]');
    for (const [name, desc] of scales) {
      scalesGroup.appendChild(makeChip(name, name, desc));
    }

    const opsGroup = this.querySelector('[data-group="ops"]');
    for (const [op, desc] of ops) {
      opsGroup.appendChild(makeChip(op, " " + op + " ", desc));
    }

    const condGroup = this.querySelector('[data-group="cond"]');
    for (const [label, insert, desc] of cond) {
      condGroup.appendChild(makeChip(label, insert, desc));
    }
  }

  _insertText(text) {
    const inp = this._input;
    const start = inp.selectionStart;
    const end = inp.selectionEnd;
    const val = inp.value;
    inp.value = val.slice(0, start) + text + val.slice(end);
    const newPos = start + text.length;
    inp.setSelectionRange(newPos, newPos);
    inp.focus();
    this._validate();
  }

  _validate() {
    const src = this._input.value;
    const { error } = compileExpr(src);
    this._error.textContent = error || "";
    this._input.classList.toggle("expr-invalid", !!error);

    // Show note hint for plain numbers on note fields
    if (this._isNoteField && !error) {
      const n = parseInt(src, 10);
      this._hint.textContent = (String(n) === src.trim() && n >= 0 && n <= 127) ? noteName(n) : "";
    } else {
      this._hint.textContent = "";
    }

    // Highlight active key on keyboard
    this._keyboard.querySelectorAll(".ee-key-active").forEach(k => k.classList.remove("ee-key-active"));
    if (this._isNoteField && !error) {
      const n = parseInt(src, 10);
      if (String(n) === src.trim() && n >= KEYBOARD_START && n <= KEYBOARD_END) {
        const key = this._keyboard.querySelector(`[data-note="${n}"]`);
        if (key) key.classList.add("ee-key-active");
      }
    }
  }

  _apply() {
    const { error } = compileExpr(this._input.value);
    if (error) return; // Don't apply invalid expressions

    if (this._onApply) {
      this._onApply(this._input.value);
    }
    this.close();
  }

  /**
   * Open the editor for a specific expression field.
   * @param {Object} opts
   * @param {string} opts.title - Modal title (e.g., "Button #1 — Note")
   * @param {string} opts.value - Current expression value
   * @param {boolean} opts.isNote - Whether this is a note field (shows keyboard)
   * @param {Function} opts.onApply - Callback with new expression string
   */
  open({ title, value, isNote, onApply }) {
    this._title.textContent = title;
    this._input.value = value || "";
    this._isNoteField = isNote;
    this._onApply = onApply;
    this._keyboardSection.style.display = isNote ? "" : "none";

    this.classList.add("open");
    document.body.classList.add("modal-open");
    this._validate();

    // Focus input and scroll keyboard to show relevant note
    requestAnimationFrame(() => {
      this._input.focus();
      this._input.setSelectionRange(this._input.value.length, this._input.value.length);

      // Scroll keyboard to show the current note if it's a plain number
      if (isNote) {
        const n = parseInt(value, 10);
        if (String(n) === (value || "").trim() && n >= KEYBOARD_START && n <= KEYBOARD_END) {
          const key = this._keyboard.querySelector(`[data-note="${n}"]`);
          if (key) {
            const scroll = this.querySelector(".ee-keyboard-scroll");
            scroll.scrollLeft = key.offsetLeft - scroll.clientWidth / 2 + key.offsetWidth / 2;
          }
        }
      }
    });
  }

  close() {
    this.classList.remove("open");
    this._onApply = null;
    // Only remove scroll lock if no ref-modals are open
    if (!document.querySelector("ref-modal.open")) {
      document.body.classList.remove("modal-open");
    }
  }

  get isOpen() {
    return this.classList.contains("open");
  }
}

customElements.define("expr-editor", ExprEditor);
