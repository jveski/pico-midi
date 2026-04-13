//! LA-2A–style electro-optical compressor using fixed-point arithmetic.
//!
//! Models the Teletronix LA-2A's signature behaviour:
//!
//! - **T4B photocell emulation**: The heart of the LA-2A sound. The
//!   electro-optical attenuator has program-dependent timing — the LED
//!   responds quickly to transients (fast attack) but the photoresistor
//!   releases with a characteristic dual-time-constant curve: a quick
//!   initial recovery (~60 ms to 50%) followed by a long, slow tail
//!   (1–5 s to full recovery). The slow time constant adapts based on
//!   how long and how hard compression has been active ("optical memory").
//!
//! - **Soft-knee compression**: The ratio is not fixed. At low levels
//!   around the threshold, ratio starts at ~1.5:1 and increases to ~3:1
//!   for heavy signals in Compress mode, or approaches ∞:1 in Limit mode.
//!
//! - **Two-control interface**: Just like the hardware —
//!   *Peak Reduction* (threshold/amount) and *Gain* (makeup).
//!   A Compress/Limit mode switch changes the knee shape.
//!
//! All DSP uses Q15 fixed-point (i16) for audio samples and Q16 (i32)
//! for envelope/gain calculations, matching the project's convention for
//! efficient RP2350 Cortex-M33 processing without FPU.
//!
//! ## Signal flow
//!
//! ```text
//!   input ──► sidechain RMS ──► gain computer (soft knee) ──► T4B photocell
//!                                                                │
//!   input ──────────────────────► gain stage (multiply) ◄────────┘
//!                                        │
//!                                   makeup gain
//!                                        │
//!                                      output
//! ```
//!
//! ## References
//!
//! - Giannoulis, Massberg & Reiss, "Digital Dynamic Range Compressor Design",
//!   JAES 2012 — soft-knee gain computer
//! - Massberg, "Digital Modeling of the Teletronix LA-2A", DAFx-2012 —
//!   T4B photocell dual-τ release model with adaptive memory
//! - UA technical docs on LA-2A circuit topology

use crate::synth::SAMPLE_RATE;

// ---------------------------------------------------------------------------
// Fixed-point helpers (local to this module)
// ---------------------------------------------------------------------------

const Q15_ONE: i32 = 1 << 15;
const Q15_MAX: i16 = i16::MAX;
const Q15_MIN: i16 = i16::MIN;

/// Multiply two Q15 values, returning Q15.
#[inline]
fn q15_mul(a: i16, b: i16) -> i16 {
    let r = (i32::from(a) * i32::from(b) + (1 << 14)) >> 15;
    r.clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16
}

// ---------------------------------------------------------------------------
// T4B Photocell model — the core of the LA-2A character
// ---------------------------------------------------------------------------

/// Models the T4B electro-optical attenuator's timing characteristics.
///
/// The LED responds quickly (fast attack ~10 ms), but the photoresistor
/// has a dual-time-constant release:
/// - **Fast release**: ~60 ms (initial 50% recovery)
/// - **Slow release**: ~1–5 seconds (long tail to full recovery)
///
/// The slow release time constant adapts: longer/harder compression leads
/// to a longer slow release (the "optical memory" effect). This is what
/// makes the LA-2A sound so musical — it naturally adapts to the program
/// material without pumping.
struct T4bPhotocell {
    /// Current gain reduction level in Q16 (0 = no reduction, Q16_ONE = full).
    envelope: i32,
    /// Accumulator tracking how long/hard compression has been active.
    /// Controls the adaptive slow release time constant. In Q16.
    compression_memory: i32,

    // Pre-computed smoothing coefficients (set by `set_params`).
    /// Attack coefficient: 1 - e^(-1 / (attack_time * sample_rate)), Q16.
    attack_coeff: i32,
    /// Fast release coefficient, Q16.
    release_fast_coeff: i32,
    /// Slow release base coefficient (adapted by memory), Q16.
    release_slow_base: i32,
    /// Memory accumulation rate, Q16.
    memory_charge_rate: i32,
    /// Memory decay rate, Q16.
    memory_discharge_rate: i32,
}

/// Q16 unit value.
const Q16_ONE: i32 = 1 << 16;

/// Compute a one-pole smoothing coefficient: α = 1 - e^(-1/(τ·fs))
/// Approximated as α ≈ 1/(τ·fs) for small values, which is accurate
/// enough for our time constants relative to the sample rate.
///
/// Returns a Q16 coefficient.
const fn time_constant_coeff(tau_ms: u32, sample_rate: u32) -> i32 {
    // α ≈ 1000 / (tau_ms * sample_rate)
    // In Q16: α_q16 = 1000 * 65536 / (tau_ms * sample_rate)
    if tau_ms == 0 {
        return Q16_ONE;
    }
    let num = 1000u64 * 65536;
    let den = tau_ms as u64 * sample_rate as u64;
    if den == 0 {
        return Q16_ONE;
    }
    let result = num / den;
    if result > Q16_ONE as u64 {
        Q16_ONE
    } else if result == 0 {
        1 // ensure non-zero
    } else {
        result as i32
    }
}

impl T4bPhotocell {
    const fn new() -> Self {
        Self {
            envelope: 0,
            compression_memory: 0,
            attack_coeff: time_constant_coeff(10, SAMPLE_RATE), // ~10 ms attack
            release_fast_coeff: time_constant_coeff(60, SAMPLE_RATE), // ~60 ms fast release
            release_slow_base: time_constant_coeff(2000, SAMPLE_RATE), // ~2 s slow release base
            memory_charge_rate: time_constant_coeff(500, SAMPLE_RATE), // memory builds over ~500 ms
            memory_discharge_rate: time_constant_coeff(5000, SAMPLE_RATE), // memory fades over ~5 s
        }
    }

    /// Process one sample of the desired gain reduction through the
    /// photocell model, returning the smoothed gain reduction in Q16.
    ///
    /// `target` is the instantaneous desired gain reduction (0..Q16_ONE)
    /// from the gain computer.
    #[inline]
    fn process(&mut self, target: i32) -> i32 {
        if target > self.envelope {
            // ── Attack phase ──
            // LED illumination: fast, ~10ms.
            let diff = target - self.envelope;
            self.envelope += ((diff as i64 * self.attack_coeff as i64) >> 16) as i32;

            // Build up compression memory while compressing
            let mem_target = Q16_ONE;
            let mem_diff = mem_target - self.compression_memory;
            self.compression_memory +=
                ((mem_diff as i64 * self.memory_charge_rate as i64) >> 16) as i32;
        } else {
            // ── Release phase ──
            // Photoresistor recovery: dual time constant.
            //
            // The release is a weighted blend of fast and slow:
            //   effective_coeff = fast_coeff * (1 - memory) + slow_coeff * memory
            //
            // When memory is high (long/hard compression), the slow time
            // constant dominates → long, musical release tail.
            // When memory is low (brief transient), the fast release
            // dominates → quick recovery.

            // Adaptive slow coefficient: scale the base by memory level.
            // More memory → even slower release (down to ~5s for sustained signals).
            let mem_factor =
                (Q16_ONE as i64 + self.compression_memory as i64).min(2 * Q16_ONE as i64) as i32;
            let slow_coeff =
                ((self.release_slow_base as i64 * Q16_ONE as i64) / mem_factor as i64) as i32;

            // Blend: effective = lerp(fast, slow, memory_normalized)
            let mem_norm = self.compression_memory.min(Q16_ONE);
            let fast_weight = Q16_ONE - mem_norm;
            let effective_coeff = ((self.release_fast_coeff as i64 * fast_weight as i64
                + slow_coeff as i64 * mem_norm as i64)
                >> 16) as i32;

            let diff = self.envelope - target;
            let step = ((diff as i64 * effective_coeff as i64) >> 16) as i32;
            self.envelope -= step.max(1); // ensure we always move toward target

            // Discharge compression memory during release
            let mem_step =
                ((self.compression_memory as i64 * self.memory_discharge_rate as i64) >> 16) as i32;
            self.compression_memory = (self.compression_memory - mem_step.max(1)).max(0);
        }

        self.envelope = self.envelope.clamp(0, Q16_ONE);
        self.compression_memory = self.compression_memory.clamp(0, Q16_ONE);

        self.envelope
    }

    #[allow(dead_code)]
    fn clear(&mut self) {
        self.envelope = 0;
        self.compression_memory = 0;
    }
}

// ---------------------------------------------------------------------------
// Gain computer — soft-knee compression curve
// ---------------------------------------------------------------------------

/// Compute the desired gain reduction for a given signal level.
///
/// Implements a soft-knee compressor curve in the **linear domain**:
/// - Below threshold: no reduction (1:1)
/// - In the knee region: smooth quadratic transition
/// - Above threshold: fixed ratio (compress ≈ 3:1, limit ≈ 20:1)
///
/// Operating in the linear domain (rather than dB) produces a slightly
/// more aggressive compression curve than a textbook dB-domain design,
/// which contributes to the characterful, coloured sound of this
/// compressor — closer to opto compressor behaviour where the
/// photoresistor response is inherently non-logarithmic.
///
/// All values in Q16. `level` is the detected signal level,
/// `threshold` is the compression onset point.
///
/// Returns gain reduction amount in Q16 (0 = no reduction).
#[inline]
fn gain_computer(level_q16: i32, threshold_q16: i32, ratio_q16: i32, knee_q16: i32) -> i32 {
    if level_q16 <= 0 {
        return 0;
    }

    // Guard against zero/negative ratio (would cause division by zero)
    let ratio_q16 = ratio_q16.max(1);

    // How far above threshold the signal is
    let overshoot = level_q16 - threshold_q16;

    if overshoot <= -(knee_q16 / 2) {
        // Below the knee: no compression
        0
    } else if overshoot >= knee_q16 / 2 {
        // Above the knee: full ratio compression
        // gain_reduction = overshoot * (1 - 1/ratio)
        // In Q16: overshoot * (Q16_ONE - Q16_ONE * Q16_ONE / ratio) / Q16_ONE
        let inv_ratio = ((Q16_ONE as i64 * Q16_ONE as i64) / ratio_q16 as i64) as i32;
        let factor = Q16_ONE - inv_ratio;
        ((overshoot as i64 * factor as i64) >> 16) as i32
    } else {
        // In the knee: smooth quadratic transition
        // gain_reduction = (overshoot + knee/2)^2 * (1 - 1/ratio) / (2 * knee)
        let x = overshoot + knee_q16 / 2;
        let inv_ratio = ((Q16_ONE as i64 * Q16_ONE as i64) / ratio_q16 as i64) as i32;
        let factor = Q16_ONE - inv_ratio;
        let x_sq = ((x as i64 * x as i64) >> 16) as i32;
        let numerator = ((x_sq as i64 * factor as i64) >> 16) as i32;
        let knee_2 = knee_q16.max(1) * 2;
        ((numerator as i64 * Q16_ONE as i64) / knee_2 as i64) as i32
    }
}

// ---------------------------------------------------------------------------
// LA-2A Compressor — public API
// ---------------------------------------------------------------------------

/// Complete LA-2A–style compressor processor.
///
/// Processes stereo sample pairs: the sidechain detects the level from
/// both channels, computes a single gain reduction value, and applies
/// it identically to both channels for coherent stereo imaging.
pub struct La2aCompressor {
    /// T4B photocell model for smooth, musical gain reduction.
    photocell: T4bPhotocell,
    /// RMS-like level detector state (Q16). Uses a one-pole smoothing
    /// filter on the squared signal magnitude for program-dependent
    /// level tracking.
    level_state: i32,
    /// Level detector smoothing coefficient (attack), Q16.
    level_attack_coeff: i32,
    /// Level detector smoothing coefficient (release), Q16.
    level_release_coeff: i32,

    // Parameters (set by `set_params`):
    /// Threshold level in Q16. Lower = more compression (Peak Reduction).
    threshold: i32,
    /// Compression ratio in Q16 (e.g., 3 * Q16_ONE for 3:1).
    ratio: i32,
    /// Soft-knee width in Q16.
    knee: i32,
    /// Makeup gain in Q16. Compensates for the gain reduction.
    /// Range: Q16_ONE (unity) to 16 * Q16_ONE (~+24 dB).
    makeup_gain: i32,
    /// Dry/wet mix for the compressor (Q15). 0 = bypass, Q15_MAX = full.
    mix: i16,
}

impl La2aCompressor {
    pub const fn new() -> Self {
        Self {
            photocell: T4bPhotocell::new(),
            level_state: 0,
            // Level detector: ~5 ms attack, ~50 ms release for RMS-like behaviour
            level_attack_coeff: time_constant_coeff(5, SAMPLE_RATE),
            level_release_coeff: time_constant_coeff(50, SAMPLE_RATE),
            threshold: Q16_ONE / 4,
            ratio: 3 * Q16_ONE,   // 3:1 (compress mode)
            knee: Q16_ONE / 4,    // Moderate soft knee
            makeup_gain: Q16_ONE, // Unity gain (no makeup) until set_params() is called
            mix: Q15_MAX,
        }
    }

    /// Set compressor parameters from 0–127 MIDI-style values.
    ///
    /// - `peak_reduction`: 0–127. Controls threshold (0 = no compression,
    ///   127 = maximum compression). Maps to threshold inversely:
    ///   higher peak reduction = lower threshold = more compression.
    /// - `gain`: 0–127. Makeup gain (0 = no gain, 127 = +24 dB equivalent).
    /// - `mode`: 0 = Compress (~3:1, wide soft knee),
    ///   1 = Limit (~20:1, narrow knee for brick-wall limiting).
    pub fn set_params(&mut self, peak_reduction: u8, gain: u8, mode: u8) {
        // Peak Reduction → threshold mapping:
        // 0   → threshold at full scale (no compression)
        // 127 → threshold near zero (maximum compression)
        // Use an inverted curve so the control feels natural.
        let pr = i32::from(peak_reduction).clamp(0, 127);
        // Threshold: from Q16_ONE (pr=0) down to Q16_ONE/32 (pr=127)
        // Exponential-ish mapping: threshold = Q16_ONE * (128 - pr)^2 / 128^2
        let inv_pr = 128 - pr;
        self.threshold = ((inv_pr * inv_pr) as i64 * Q16_ONE as i64 / (128 * 128) as i64) as i32;
        self.threshold = self.threshold.max(Q16_ONE / 64); // Floor to prevent div-by-zero-like issues

        // Mode: compress vs limit
        match mode {
            0 => {
                // Compress: ~3:1 ratio, wide soft knee (classic LA-2A)
                self.ratio = 3 * Q16_ONE;
                self.knee = Q16_ONE / 4;
            }
            _ => {
                // Limit: ~20:1, narrow knee (aggressive limiting)
                self.ratio = 20 * Q16_ONE;
                self.knee = Q16_ONE / 16;
            }
        }

        // Makeup gain: 0–127 → 0 dB to ~+24 dB
        // Linear mapping: gain_linear = 1.0 + (gain_midi / 127) * 15.0
        // (16x max = ~+24 dB)
        // In Q16: Q16_ONE + gain * 15 * Q16_ONE / 127
        let g = i32::from(gain).clamp(0, 127);
        let makeup = Q16_ONE + g * 15 * Q16_ONE / 127;
        self.makeup_gain = makeup.clamp(Q16_ONE, 16 * Q16_ONE);
    }

    /// Set the dry/wet mix (0–127). 0 = fully bypass, 127 = fully compressed.
    pub fn set_mix(&mut self, mix: u8) {
        let m = i32::from(mix).clamp(0, 127);
        self.mix = (m * i32::from(Q15_MAX) / 127).clamp(0, i32::from(Q15_MAX)) as i16;
    }

    /// Process a stereo sample pair through the compressor.
    ///
    /// The sidechain takes the maximum of both channels for level
    /// detection, then applies identical gain reduction to both
    /// channels for coherent stereo imaging.
    pub fn process(&mut self, left: i16, right: i16) -> (i16, i16) {
        // ── 1. Sidechain level detection ──
        // Take peak of both channels (absolute values)
        let abs_l = (left as i32).unsigned_abs() as i32;
        let abs_r = (right as i32).unsigned_abs() as i32;
        let peak = abs_l.max(abs_r);

        // Convert to Q16 level (0..Q16_ONE range)
        // Input range is 0..32768 (absolute Q15), scale to Q16
        let input_level = ((peak as i64 * Q16_ONE as i64) >> 15) as i32;

        // Smooth the level with asymmetric attack/release
        let coeff = if input_level > self.level_state {
            self.level_attack_coeff
        } else {
            self.level_release_coeff
        };
        let diff = input_level - self.level_state;
        self.level_state += ((diff as i64 * coeff as i64) >> 16) as i32;
        self.level_state = self.level_state.clamp(0, Q16_ONE);

        // ── 2. Gain computer ──
        let target_reduction =
            gain_computer(self.level_state, self.threshold, self.ratio, self.knee);
        let target_reduction = target_reduction.clamp(0, Q16_ONE);

        // ── 3. T4B photocell smoothing ──
        let smoothed_reduction = self.photocell.process(target_reduction);

        // ── 4. Convert gain reduction to gain multiplier ──
        // gain = 1.0 - reduction
        // In Q15: gain = Q15_MAX - (reduction * Q15_MAX / Q16_ONE)
        let gain_q15 =
            (Q15_ONE - ((smoothed_reduction as i64 * Q15_ONE as i64) >> 16) as i32).max(0);
        let gain = gain_q15.clamp(0, i32::from(Q15_MAX)) as i16;

        // ── 5. Apply gain reduction to both channels ──
        let comp_l = q15_mul(left, gain);
        let comp_r = q15_mul(right, gain);

        // ── 6. Apply makeup gain (Q16) ──
        let makeup_l = ((i32::from(comp_l) as i64 * self.makeup_gain as i64) >> 16)
            .clamp(i32::from(Q15_MIN) as i64, i32::from(Q15_MAX) as i64)
            as i16;
        let makeup_r = ((i32::from(comp_r) as i64 * self.makeup_gain as i64) >> 16)
            .clamp(i32::from(Q15_MIN) as i64, i32::from(Q15_MAX) as i64)
            as i16;

        // ── 7. Dry/wet mix ──
        let dry = (i32::from(Q15_MAX) - i32::from(self.mix)).max(0) as i16;
        let out_l = (i32::from(q15_mul(left, dry)) + i32::from(q15_mul(makeup_l, self.mix)))
            .clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16;
        let out_r = (i32::from(q15_mul(right, dry)) + i32::from(q15_mul(makeup_r, self.mix)))
            .clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16;

        (out_l, out_r)
    }

    /// Clear compressor state. Use when toggling on/off to avoid stale state.
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.photocell.clear();
        self.level_state = 0;
    }
}
