import { BaseElement, num, updateHint } from "./helpers.js";

export class MidiChannel extends BaseElement {
  init() {
    this.querySelector("#midiChannel").addEventListener("input", () => this._updateHint());
  }

  _updateHint() {
    updateHint(this, "midiChannel", "midiChannelHint", v => "Ch " + (v + 1));
  }

  set value(v) {
    this.querySelector("#midiChannel").value = v;
    this._updateHint();
  }

  get value() {
    return num(this.querySelector("#midiChannel").value, 0);
  }
}

customElements.define("midi-channel", MidiChannel);
