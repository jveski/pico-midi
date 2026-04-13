import { BaseElement, num, clamp, toggleFieldsVisibility } from "./helpers.js";

export class SynthControl extends BaseElement {
  init() {
    this.querySelector("#synthEnabled").addEventListener("change", () => {
      this._updateVisibility();
    });
  }

  render(cfg) {
    const s = cfg.synth;
    this.querySelector("#synthEnabled").checked = s.enabled;
    this.querySelector("#synthOsc1Wave").value = s.osc1_waveform;
    this.querySelector("#synthOsc2Wave").value = s.osc2_waveform;
    this.querySelector("#synthDetune").value = s.osc2_detune_cents;
    this.querySelector("#synthSemitone").value = s.osc2_semitone;
    this.querySelector("#synthOscMix").value = s.osc_mix;
    this.querySelector("#synthFilterCutoff").value = s.filter_cutoff;
    this.querySelector("#synthFilterReso").value = s.filter_resonance;
    this.querySelector("#synthFilterEnvAmt").value = s.filter_env_amount;
    this.querySelector("#synthAmpAttack").value = s.amp_attack_ms;
    this.querySelector("#synthAmpDecay").value = s.amp_decay_ms;
    this.querySelector("#synthAmpSustain").value = s.amp_sustain_pct;
    this.querySelector("#synthAmpRelease").value = s.amp_release_ms;
    this.querySelector("#synthFilterAttack").value = s.filter_attack_ms;
    this.querySelector("#synthFilterDecay").value = s.filter_decay_ms;
    this.querySelector("#synthFilterSustain").value = s.filter_sustain_pct;
    this.querySelector("#synthFilterRelease").value = s.filter_release_ms;
    this.querySelector("#synthVolume").value = s.master_volume;
    this._updateVisibility();
  }

  readFromDOM() {
    return {
      enabled: this.querySelector("#synthEnabled").checked,
      audio_pin: 14, // Only GP14 is currently supported
      osc1_waveform: clamp(num(this.querySelector("#synthOsc1Wave").value, 0), 0, 3),
      osc2_waveform: clamp(num(this.querySelector("#synthOsc2Wave").value, 0), 0, 3),
      osc2_detune_cents: clamp(num(this.querySelector("#synthDetune").value, 7), -50, 50),
      osc2_semitone: clamp(num(this.querySelector("#synthSemitone").value, 0), -24, 24),
      osc_mix: clamp(num(this.querySelector("#synthOscMix").value, 64), 0, 127),
      filter_cutoff: clamp(num(this.querySelector("#synthFilterCutoff").value, 80), 0, 127),
      filter_resonance: clamp(num(this.querySelector("#synthFilterReso").value, 40), 0, 127),
      filter_env_amount: clamp(num(this.querySelector("#synthFilterEnvAmt").value, 64), 0, 127),
      amp_attack_ms: clamp(num(this.querySelector("#synthAmpAttack").value, 10), 0, 5000),
      amp_decay_ms: clamp(num(this.querySelector("#synthAmpDecay").value, 200), 0, 5000),
      amp_sustain_pct: clamp(num(this.querySelector("#synthAmpSustain").value, 70), 0, 100),
      amp_release_ms: clamp(num(this.querySelector("#synthAmpRelease").value, 300), 0, 5000),
      filter_attack_ms: clamp(num(this.querySelector("#synthFilterAttack").value, 5), 0, 5000),
      filter_decay_ms: clamp(num(this.querySelector("#synthFilterDecay").value, 300), 0, 5000),
      filter_sustain_pct: clamp(num(this.querySelector("#synthFilterSustain").value, 30), 0, 100),
      filter_release_ms: clamp(num(this.querySelector("#synthFilterRelease").value, 200), 0, 5000),
      master_volume: clamp(num(this.querySelector("#synthVolume").value, 80), 0, 127),
    };
  }

  _updateVisibility() {
    toggleFieldsVisibility(this, "synthEnabled", "synthFields");
  }
}

customElements.define("synth-control", SynthControl);
