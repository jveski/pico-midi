export class ConfigPanel extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.id = "configPanel";
    this.style.display = "none";

    this.innerHTML =
      // General
      '<collapsible-card data-section="general" data-title="General">' +
        '<midi-channel></midi-channel>' +
      '</collapsible-card>' +

      // Buttons
      '<collapsible-card data-section="buttons" data-title="Buttons" data-badge-id="btnCount">' +
        '<item-list data-type="button" data-max="8" data-list-id="buttonList" data-count-id="btnCount" data-add-id="addButton" data-add-label="+ Add Button"></item-list>' +
      '</collapsible-card>' +

      // Touch Pads
      '<collapsible-card data-section="touch" data-title="Touch Pads" data-badge-id="touchCount">' +
        '<item-list data-type="touch" data-max="8" data-list-id="touchList" data-count-id="touchCount" data-add-id="addTouch" data-add-label="+ Add Touch Pad"></item-list>' +
      '</collapsible-card>' +

      // Pots
      '<collapsible-card data-section="pots" data-title="Potentiometers" data-badge-id="potCount">' +
        '<item-list data-type="pot" data-max="4" data-list-id="potList" data-count-id="potCount" data-add-id="addPot" data-add-label="+ Add Pot"></item-list>' +
      '</collapsible-card>' +

      // LDR
      '<collapsible-card data-section="ldr" data-title="LDR (Light Sensor)">' +
        '<ldr-section></ldr-section>' +
      '</collapsible-card>' +

      // Accelerometer
      '<collapsible-card data-section="accel" data-title="Accelerometer">' +
        '<accel-section></accel-section>' +
      '</collapsible-card>' +

      // Expression Reference
      '<collapsible-card data-title="Expression Reference" data-collapsed>' +
        '<div class="expr-ref">' +
          '<p>The <strong>Note</strong> and <strong>Velocity</strong> fields for buttons and touch pads accept <strong>expressions</strong>. ' +
            'An expression can be a plain number, a sensor variable, or a formula combining both.</p>' +

          '<h3>Variables</h3>' +
          '<table>' +
            '<tr><th>Variable</th><th>Description</th></tr>' +
            '<tr><td><code>pot0</code> &ndash; <code>pot3</code></td><td>Potentiometer values (0&ndash;127)</td></tr>' +
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
  }

  get buttonList() { return this.querySelector('item-list[data-type="button"]'); }
  get touchList() { return this.querySelector('item-list[data-type="touch"]'); }
  get potList() { return this.querySelector('item-list[data-type="pot"]'); }
  get ldrSection() { return this.querySelector("ldr-section"); }
  get accelSection() { return this.querySelector("accel-section"); }
  get midiChannel() { return this.querySelector("midi-channel"); }
}

customElements.define("config-panel", ConfigPanel);
