export class ConfigPanel extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.id = "configPanel";
    this.style.display = "none";

    this.innerHTML =
      // Buttons
      '<collapsible-card data-section="buttons" data-title="Buttons" data-badge-id="btnCount">' +
        '<item-list data-type="button" data-list-id="buttonList" data-count-id="btnCount"></item-list>' +
      '</collapsible-card>' +

      // Touch Pads
      '<collapsible-card data-section="touch" data-title="Touch Pads" data-badge-id="touchCount">' +
        '<item-list data-type="touch" data-list-id="touchList" data-count-id="touchCount"></item-list>' +
      '</collapsible-card>' +

      // Pots
      '<collapsible-card data-section="pots" data-title="Potentiometers" data-badge-id="potCount">' +
        '<item-list data-type="pot" data-list-id="potList" data-count-id="potCount"></item-list>' +
      '</collapsible-card>' +

      // LDR
      '<collapsible-card data-section="ldr" data-title="LDR (Light Sensor)">' +
        '<ldr-section></ldr-section>' +
      '</collapsible-card>' +

      // Accelerometer
      '<collapsible-card data-section="accel" data-title="Accelerometer">' +
        '<accel-section></accel-section>' +
      '</collapsible-card>' +

      // General
      '<collapsible-card data-section="general" data-title="General">' +
        '<midi-channel></midi-channel>' +
        '<div class="project-actions-inline">' +
          '<h3>Project</h3>' +
          '<div class="project-actions">' +
            '<button class="btn" id="btnExport">Export Project</button>' +
            '<button class="btn" id="btnImport">Import Project</button>' +
            '<button class="btn" id="btnReset">Reset Defaults</button>' +
          '</div>' +
          '<input type="file" id="importFile" accept=".json" style="display:none">' +
        '</div>' +
      '</collapsible-card>' +

      // Expression Reference
      '<collapsible-card data-title="Expression Reference" data-collapsed>' +
        '<div class="expr-ref">' +
          '<p>The <strong>Note</strong> and <strong>Velocity</strong> fields for buttons and touch pads accept <strong>expressions</strong>. ' +
            'An expression can be a plain number, a sensor variable, or a formula combining both.</p>' +

          '<h3>Variables</h3>' +
          '<table>' +
            '<tr><th>Variable</th><th>Description</th></tr>' +
            '<tr><td><code>pot0</code> &ndash; <code>pot1</code></td><td>Potentiometer values (0&ndash;127)</td></tr>' +
            '<tr><td><code>ldr</code></td><td>Light sensor value (0&ndash;127)</td></tr>' +
            '<tr><td><code>accel_x</code></td><td>Accelerometer X-axis (0&ndash;127)</td></tr>' +
            '<tr><td><code>accel_y</code></td><td>Accelerometer Y-axis (0&ndash;127)</td></tr>' +
          '</table>' +

          '<h3>Operators</h3>' +
          '<table>' +
            '<tr><th>Operator</th><th>Description</th></tr>' +
            '<tr><td><code>+</code></td><td>Add (saturates at 127)</td></tr>' +
            '<tr><td><code>-</code></td><td>Subtract (saturates at 0)</td></tr>' +
            '<tr><td><code>*</code></td><td>Multiply (saturates at 127)</td></tr>' +
            '<tr><td><code>/</code></td><td>Integer divide</td></tr>' +
          '</table>' +

          '<h3>Functions</h3>' +
          '<table>' +
            '<tr><th>Function</th><th>Description</th></tr>' +
            '<tr><td><code>min(a, b)</code></td><td>Minimum of two values</td></tr>' +
            '<tr><td><code>max(a, b)</code></td><td>Maximum of two values</td></tr>' +
            '<tr><td><code>clamp(val, lo, hi)</code></td><td>Clamp val to range [lo, hi]</td></tr>' +
            '<tr><td><code>lerp(a, b, t)</code></td><td>Interpolate from a to b (t: 0&ndash;127)</td></tr>' +
          '</table>' +

          '<h3>Conditionals</h3>' +
          '<p><code>a &gt; b ? x : y</code> &mdash; if a is greater than b, use x; otherwise use y.</p>' +

          '<h3>Examples</h3>' +
          '<ul class="expr-examples">' +
            '<li><code>60</code> &mdash; fixed note: middle C</li>' +
            '<li><code>pot0</code> &mdash; note follows potentiometer 0</li>' +
            '<li><code>pot0 + 24</code> &mdash; pot value shifted up 2 octaves</li>' +
            '<li><code>pot0 &gt; 64 ? 72 : 60</code> &mdash; C5 when pot is above halfway, else C4</li>' +
            '<li><code>min(pot0, 100)</code> &mdash; pot value capped at 100</li>' +
            '<li><code>clamp(pot0, 20, 100)</code> &mdash; pot value restricted to 20&ndash;100</li>' +
            '<li><code>lerp(36, 84, pot0)</code> &mdash; pot sweeps from C2 to C6</li>' +
          '</ul>' +

          '<h3>Notes</h3>' +
          '<ul class="expr-notes">' +
            '<li>All values are integers in the range 0&ndash;127</li>' +
            '<li>Arithmetic saturates (never wraps around)</li>' +
            '<li>Division by zero returns 127</li>' +
            '<li>Expressions compile to a max of 16 bytes of bytecode</li>' +
            '<li>Parentheses <code>()</code> can be used for grouping</li>' +
           '</ul>' +
        '</div>' +
      '</collapsible-card>';

    this._wireProjectActions();
  }

  _wireProjectActions() {
    this.querySelector("#btnImport").addEventListener("click", () => {
      this.querySelector("#importFile").click();
    });

    this.querySelector("#importFile").addEventListener("change", (e) => {
      const file = e.target.files[0];
      if (file) {
        this.dispatchEvent(new CustomEvent("project-import", { detail: { file }, bubbles: true }));
      }
      e.target.value = "";
    });
  }

  get buttonList() { return this.querySelector('item-list[data-type="button"]'); }
  get touchList() { return this.querySelector('item-list[data-type="touch"]'); }
  get potList() { return this.querySelector('item-list[data-type="pot"]'); }
  get ldrSection() { return this.querySelector("ldr-section"); }
  get accelSection() { return this.querySelector("accel-section"); }
  get midiChannel() { return this.querySelector("midi-channel"); }
  get btnExport() { return this.querySelector("#btnExport"); }
  get btnImport() { return this.querySelector("#btnImport"); }
  get btnReset() { return this.querySelector("#btnReset"); }

  set projectBusy(v) {
    this.btnExport.disabled = v;
    this.btnImport.disabled = v;
    this.btnReset.disabled = v;
  }
}

customElements.define("config-panel", ConfigPanel);
