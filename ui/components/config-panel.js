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
