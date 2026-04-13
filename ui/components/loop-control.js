import { BaseElement, num, clamp, toggleFieldsVisibility, readClamped } from "./helpers.js";

const QUANTIZE_LABELS = ["Off", "1/4", "1/8", "1/16"];
const LAYER_STATE_LABELS = ["Empty", "Rec", "Play", "Muted"];
const LAYER_STATE_CLASSES = ["empty", "recording", "playing", "muted"];

export class LoopControl extends BaseElement {
  init() {
    this.querySelector("#loopEnabled").addEventListener("change", () => {
      this._updateVisibility();
    });
  }

  render(cfg) {
    this.querySelector("#loopEnabled").checked = cfg.loop_cfg.enabled;
    this.querySelector("#loopBpm").value = cfg.loop_cfg.bpm;
    this.querySelector("#loopBars").value = cfg.loop_cfg.bars;
    this.querySelector("#loopQuantize").value = cfg.loop_cfg.quantize;
    this.querySelector("#loopNumLayers").value = cfg.loop_cfg.num_layers;
    this._updateVisibility();
    this._buildLayerLanes(cfg.loop_cfg.num_layers);
  }

  readFromDOM() {
    return {
      enabled: this.querySelector("#loopEnabled").checked,
      num_layers: clamp(num(this.querySelector("#loopNumLayers").value, 4), 2, 4),
      bpm: clamp(num(this.querySelector("#loopBpm").value, 120), 40, 240),
      quantize: clamp(num(this.querySelector("#loopQuantize").value, 2), 0, 3),
      bars: clamp(num(this.querySelector("#loopBars").value, 4), 1, 8),
    };
  }

  _updateVisibility() {
    toggleFieldsVisibility(this, "loopEnabled", "loopFields");
  }

  _buildLayerLanes(numLayers) {
    const container = this.querySelector("#loopLayers");
    container.innerHTML = "";
    for (let i = 0; i < numLayers; i++) {
      const lane = document.createElement("div");
      lane.className = "loop-layer-lane";
      lane.dataset.layer = i;
      lane.innerHTML = `
        <span class="loop-layer-label">L${i + 1}</span>
        <span class="loop-layer-state" id="loopLayerState${i}">Empty</span>
        <span class="loop-layer-events" id="loopLayerEvents${i}">0</span>
        <button class="btn btn-sm loop-layer-btn loop-rec-btn" data-action="record" data-layer="${i}" title="Record">REC</button>
        <button class="btn btn-sm loop-layer-btn loop-mute-btn" data-action="mute" data-layer="${i}" title="Mute">MUTE</button>
        <button class="btn btn-sm loop-layer-btn loop-clear-btn" data-action="clear" data-layer="${i}" title="Clear">CLR</button>
      `;
      container.appendChild(lane);
    }
  }

  /** Update live loop state from the device monitor. */
  applyLoopState(state) {
    if (!state) return;
    const progress = this.querySelector("#loopProgress");
    if (progress) progress.style.width = ((state.progress / 255) * 100).toFixed(1) + "%";

    const playBtn = this.querySelector('[data-action="play"]');
    const stopBtn = this.querySelector('[data-action="stop"]');
    if (playBtn) playBtn.classList.toggle("active", state.playing);
    if (stopBtn) stopBtn.classList.toggle("active", !state.playing);

    for (let i = 0; i < state.num_layers; i++) {
      const stateEl = this.querySelector(`#loopLayerState${i}`);
      const eventsEl = this.querySelector(`#loopLayerEvents${i}`);
      const lane = this.querySelector(`.loop-layer-lane[data-layer="${i}"]`);
      const s = state.layer_states[i];

      if (stateEl) stateEl.textContent = LAYER_STATE_LABELS[s] || "?";
      if (eventsEl) eventsEl.textContent = state.layer_event_counts[i];
      if (lane) {
        // Remove all state classes, then add the current one
        lane.classList.remove(...LAYER_STATE_CLASSES);
        if (LAYER_STATE_CLASSES[s]) lane.classList.add(LAYER_STATE_CLASSES[s]);
      }

      // Toggle rec button active state
      const recBtn = this.querySelector(`[data-action="record"][data-layer="${i}"]`);
      if (recBtn) recBtn.classList.toggle("active", s === 1); // Recording

      // Toggle mute button active state
      const muteBtn = this.querySelector(`[data-action="mute"][data-layer="${i}"]`);
      if (muteBtn) muteBtn.classList.toggle("active", s === 3); // Muted
    }
  }
}

customElements.define("loop-control", LoopControl);
