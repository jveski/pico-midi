export class MidiChannel extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;
    this.innerHTML =
      '<div class="field">' +
        '<label>MIDI Channel</label>' +
        '<input type="number" id="midiChannel" min="0" max="15" value="0">' +
        '<span class="note-hint" id="midiChannelHint">Ch 1</span>' +
      '</div>';

    this.querySelector("#midiChannel").addEventListener("input", () => this._updateHint());
  }

  _updateHint() {
    const v = parseInt(this.querySelector("#midiChannel").value, 10);
    this.querySelector("#midiChannelHint").textContent = isNaN(v) ? "" : "Ch " + (v + 1);
  }

  set value(v) {
    this.querySelector("#midiChannel").value = v;
    this._updateHint();
  }

  get value() {
    return parseInt(this.querySelector("#midiChannel").value, 10);
  }
}

customElements.define("midi-channel", MidiChannel);
