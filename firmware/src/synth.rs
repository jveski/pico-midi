//! Moog-style analog synthesizer engine using fixed-point arithmetic.
//!
//! This module implements a two-oscillator subtractive synth with:
//! - Band-limited oscillators (saw, square, triangle, sine) using polyBLEP
//! - Huovilainen-model 4-pole (24 dB/oct) Moog ladder filter with nonlinear saturation
//! - ADSR envelopes for amplitude and filter cutoff
//! - Oscillator drift for analog warmth
//!
//! All DSP uses Q15 fixed-point (i16 with 15 fractional bits) for audio samples
//! and Q16 fixed-point (i32 with 16 fractional bits) for intermediate calculations.
//! This keeps everything efficient on the RP2350 Cortex-M33 which has no FPU in the default configuration.

use crate::config::SynthConfig;
use crate::reverb::Freeverb;

// ---------------------------------------------------------------------------
// Fixed-point constants and helpers
// ---------------------------------------------------------------------------

/// Q15 format: 1 sign bit + 15 fractional bits. Range: -1.0 .. +0.99997
const Q15_ONE: i32 = 1 << 15; // 32768
const Q15_MAX: i16 = i16::MAX; // 32767
const Q15_MIN: i16 = i16::MIN; // -32768

/// Q16 format for wider intermediates: 1 sign bit + 15 integer + 16 fractional
const Q16_ONE: i32 = 1 << 16;

/// Audio sample rate in Hz.
pub const SAMPLE_RATE: u32 = 22_050;

/// Number of entries in the sine quarter-wave lookup table.
const SINE_TABLE_SIZE: usize = 256;

/// MIDI note 69 = A4 = 440 Hz. Phase increment table is indexed by MIDI note.
const MIDI_NOTE_COUNT: usize = 128;

// ---------------------------------------------------------------------------
// Waveform types
// ---------------------------------------------------------------------------

/// Oscillator waveform selection.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    Saw = 0,
    Square = 1,
    Triangle = 2,
    Sine = 3,
}

impl Waveform {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Saw,
            1 => Self::Square,
            2 => Self::Triangle,
            3 => Self::Sine,
            _ => Self::Saw,
        }
    }
}

// ---------------------------------------------------------------------------
// Lookup tables (computed at compile time)
// ---------------------------------------------------------------------------

/// Quarter-wave sine table in Q15. sine_table[i] = sin(i * pi/2 / 256) * 32767
/// Uses Bhaskara I's sine approximation for const evaluation.
const SINE_TABLE: [i16; SINE_TABLE_SIZE] = {
    let mut table = [0i16; SINE_TABLE_SIZE];
    let mut i = 0;
    while i < SINE_TABLE_SIZE {
        // Bhaskara I's approximation:
        // sin(x) ≈ 16x(pi - x) / (5pi^2 - 4x(pi - x)) for x in [0, pi]
        // We have theta in [0, pi/2], so pi - theta > 0 and this is well-behaved.
        let pi_scaled = 3_141_593i64;
        let theta = (i as i64) * pi_scaled / (2 * SINE_TABLE_SIZE as i64);
        let pi_minus_theta = pi_scaled - theta;
        let numerator = 16 * theta * pi_minus_theta;
        let denominator = 5 * pi_scaled * pi_scaled - 4 * theta * pi_minus_theta;
        let sin_val = if denominator == 0 {
            0i64
        } else {
            numerator * 32767 / denominator
        };
        table[i] = if sin_val > 32767 {
            32767
        } else if sin_val < 0 {
            0
        } else {
            sin_val as i16
        };
        i += 1;
    }
    table
};

/// Phase increment per sample for each MIDI note, in Q32 (u32 phase accumulator).
/// phase_inc = freq * 2^32 / SAMPLE_RATE. Computed at compile time.
const PHASE_INC_TABLE: [u32; MIDI_NOTE_COUNT] = {
    let mut table = [0u32; MIDI_NOTE_COUNT];
    let mut note = 0;
    while note < MIDI_NOTE_COUNT {
        // We compute freq for each MIDI note using semitone ratios and octave shifts.
        // 2^(semitone/12) is looked up from a Q24 table; octave offset is a bit shift.
        let semitone_within_octave = if note >= 69 {
            (note - 69) % 12
        } else {
            let diff = 69 - note;
            let m = diff % 12;
            if m == 0 {
                0
            } else {
                12 - m
            }
        };

        // Semitone frequency ratios relative to the octave base, in Q24
        let ratios_q24: [u32; 12] = [
            16_777_216, // 2^(0/12) = 1.0
            17_774_451, // 2^(1/12) ≈ 1.05946
            18_827_722, // 2^(2/12) ≈ 1.12246
            19_941_076, // 2^(3/12) ≈ 1.18921
            21_118_846, // 2^(4/12) ≈ 1.25992
            22_365_660, // 2^(5/12) ≈ 1.33484
            23_686_474, // 2^(6/12) ≈ 1.41421
            25_086_594, // 2^(7/12) ≈ 1.49831
            26_571_710, // 2^(8/12) ≈ 1.58740
            28_147_930, // 2^(9/12) ≈ 1.68179
            29_821_826, // 2^(10/12) ≈ 1.78180
            31_600_483, // 2^(11/12) ≈ 1.88775
        ];

        // Octave shift relative to A4 (note 69).
        let octave_shift: i32 = if note >= 69 {
            ((note - 69) / 12) as i32
        } else {
            let diff = 69 - note;
            let m = diff % 12;
            let octaves_down = diff / 12 + if m > 0 { 1 } else { 0 };
            -(octaves_down as i32)
        };

        // A4 = 440 Hz, in Q8: 440 * 256 = 112_640
        // freq_q8 = 112_640 * ratio / 2^24, then shifted by octave
        let base_freq_q8: u64 = (112_640u64 * ratios_q24[semitone_within_octave] as u64) >> 24;

        let freq_q8: u64 = if octave_shift >= 0 {
            base_freq_q8 << (octave_shift as u32)
        } else {
            base_freq_q8 >> ((-octave_shift) as u32)
        };

        // phase_inc = freq_q8 * 2^32 / (256 * SAMPLE_RATE)
        // = freq_q8 * 4_294_967_296 / 5_644_800
        let phase_inc_64: u64 = freq_q8 * 4_294_967_296u64 / 5_644_800;

        table[note] = if phase_inc_64 > u32::MAX as u64 {
            u32::MAX
        } else {
            phase_inc_64 as u32
        };

        note += 1;
    }
    table
};

// ---------------------------------------------------------------------------
// Fixed-point math utilities
// ---------------------------------------------------------------------------

/// Multiply two Q15 values, returning Q15.
#[inline]
fn q15_mul(a: i16, b: i16) -> i16 {
    let r = (i32::from(a) * i32::from(b) + (1 << 14)) >> 15;
    r.clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16
}

/// Multiply i32 Q16 values, returning Q16. Uses 64-bit intermediate.
#[inline]
fn q16_mul(a: i32, b: i32) -> i32 {
    let r = ((a as i64) * (b as i64) + (1 << 15)) >> 16;
    r.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// Approximate tanh(x) for x in Q16 format using the rational Padé approximant:
/// tanh(x) ≈ x * (27 + x²) / (27 + 9·x²)
/// This gives good accuracy for |x| < 3 and saturates cleanly beyond that.
///
/// All intermediate arithmetic uses i64 to avoid overflow: the numerator
/// term x·(27 + x²) can reach ~30·3·Q16_ONE² which exceeds i32 range.
#[inline]
fn tanh_q16(x: i32) -> i32 {
    let x_clamped = x.clamp(-(3 * Q16_ONE), 3 * Q16_ONE) as i64;
    let q = Q16_ONE as i64;

    // x² in Q16 (via i64 to avoid overflow)
    let x2 = (x_clamped * x_clamped + (1 << 15)) >> 16;

    // numerator = x * (27 + x²)  — result is in Q16·Q16 = Q32
    let num = x_clamped * (27 * q + x2);
    // denominator = 27 + 9·x²  — in Q16
    let den = 27 * q + 9 * x2;

    if den == 0 {
        return if x >= 0 { Q16_ONE } else { -Q16_ONE };
    }

    // num is Q32, den is Q16, so num/den gives Q16 directly
    let result = num / den;
    result.clamp(-q, q) as i32
}

/// Look up sine from the quarter-wave table. Phase is a u32 where full range = one cycle.
#[inline]
fn sine_q15(phase: u32) -> i16 {
    let quadrant = (phase >> 30) & 3;
    let index_full = (phase >> 22) & 0xFF; // 8 bits -> 256 entries

    let idx = index_full as usize;

    match quadrant {
        0 => SINE_TABLE[idx],
        1 => SINE_TABLE[255 - idx],
        2 => {
            let v = SINE_TABLE[idx];
            if v == Q15_MIN {
                Q15_MAX
            } else {
                -v
            }
        }
        _ => {
            let v = SINE_TABLE[255 - idx];
            if v == Q15_MIN {
                Q15_MAX
            } else {
                -v
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Oscillator
// ---------------------------------------------------------------------------

/// Band-limited oscillator with polyBLEP anti-aliasing.
struct Oscillator {
    phase: u32,
    phase_inc: u32,
    /// Small random drift applied to phase_inc for analog warmth.
    drift: i32,
    /// Simple LFSR for drift modulation.
    drift_lfsr: u32,
    drift_counter: u16,
}

impl Oscillator {
    const fn new() -> Self {
        Self {
            phase: 0,
            phase_inc: 0,
            drift: 0,
            drift_lfsr: 0xDEAD_BEEF,
            drift_counter: 0,
        }
    }

    fn set_frequency(&mut self, midi_note: u8, detune_cents: i16) {
        let note = (midi_note as usize).min(MIDI_NOTE_COUNT - 1);
        let base_inc = PHASE_INC_TABLE[note];

        // Apply detune: each cent is 1/1200 of an octave
        // detune_ratio ≈ 1 + cents * ln(2) / 1200 ≈ 1 + cents * 0.000578
        // In Q16: cents * 38 (since 0.000578 * 65536 ≈ 37.9)
        let detune_offset =
            ((base_inc as i64) * i64::from(detune_cents) * 38 / Q16_ONE as i64) as i32;
        self.phase_inc = (base_inc as i64 + detune_offset as i64).max(0) as u32;
    }

    /// Update oscillator drift (call at a low rate, e.g., every 64 samples).
    fn update_drift(&mut self) {
        self.drift_counter = self.drift_counter.wrapping_add(1);
        if self.drift_counter & 0x3F == 0 {
            // Simple LFSR for pseudo-random drift
            let bit = ((self.drift_lfsr >> 31) ^ (self.drift_lfsr >> 21)) & 1;
            self.drift_lfsr = (self.drift_lfsr << 1) | bit;
            // Drift range: +/- ~8 cents worth of phase increment deviation
            // 8 cents = phase_inc * 8 * 38 / 65536 ≈ phase_inc / 215
            let max_drift = (self.phase_inc / 215) as i32;
            self.drift = if self.drift_lfsr & 1 == 0 {
                max_drift
            } else {
                -max_drift
            };
        }
    }

    /// Generate one sample in Q15.
    fn tick(&mut self, waveform: Waveform) -> i16 {
        let inc = (self.phase_inc as i64 + self.drift as i64).max(1) as u32;
        self.phase = self.phase.wrapping_add(inc);

        match waveform {
            Waveform::Saw => self.saw_polyblep(inc),
            Waveform::Square => self.square_polyblep(inc),
            Waveform::Triangle => self.triangle(inc),
            Waveform::Sine => sine_q15(self.phase),
        }
    }

    /// Naive sawtooth with polyBLEP correction.
    fn saw_polyblep(&self, inc: u32) -> i16 {
        // Naive saw: phase maps linearly from -1 to +1
        let naive = ((self.phase >> 16) as i32 - 32768) as i16;

        // PolyBLEP correction at the discontinuity (phase wrap)
        let t = self.phase as i64;
        let dt = inc as i64;
        let blep = poly_blep_q15(t, dt);

        let result = i32::from(naive) - i32::from(blep);
        result.clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16
    }

    /// Square wave via two polyBLEP-corrected edges.
    fn square_polyblep(&self, inc: u32) -> i16 {
        // Naive square: +1 for first half, -1 for second half
        let naive: i16 = if self.phase < 0x8000_0000 {
            Q15_MAX
        } else {
            Q15_MIN
        };

        let t = self.phase as i64;
        let dt = inc as i64;

        // Correction at phase 0 (rising edge)
        let blep1 = poly_blep_q15(t, dt);
        // Correction at phase 0.5 (falling edge)
        let t2 = (self.phase.wrapping_add(0x8000_0000)) as i64;
        let blep2 = poly_blep_q15(t2, dt);

        let result = i32::from(naive) + i32::from(blep1) - i32::from(blep2);
        result.clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16
    }

    /// Triangle wave derived from integrated square (no aliasing issues at normal freqs).
    fn triangle(&self, _inc: u32) -> i16 {
        // Triangle: piecewise linear from phase
        // 0..0.25: 0 to +1
        // 0.25..0.75: +1 to -1
        // 0.75..1.0: -1 to 0
        let p = self.phase;
        let val = if p < 0x4000_0000 {
            // 0..0.25 -> 0..+max
            (p >> 14) as i32
        } else if p < 0xC000_0000 {
            // 0.25..0.75 -> +max..−max
            (0x1FFFF_i32) - ((p >> 14) as i32)
        } else {
            // 0.75..1.0 -> −max..0
            ((p >> 14) as i32) - 0x3FFFF_i32
        };
        // val is in range [-65536, 65535], scale to Q15 (-32768..32767)
        let scaled =
            (val * i32::from(Q15_MAX) / 0x10000).clamp(i32::from(Q15_MIN), i32::from(Q15_MAX));
        scaled as i16
    }
}

/// PolyBLEP residual in Q15. `t` is the current phase (0..2^32),
/// `dt` is the phase increment per sample.
#[inline]
fn poly_blep_q15(t: i64, dt: i64) -> i16 {
    if dt == 0 {
        return 0;
    }
    let full = 1i64 << 32;
    // t normalized to [0, 1) in Q32
    let t_norm = t & (full - 1);

    if t_norm < dt {
        // Just after discontinuity
        // t/dt in Q15
        let x = (t_norm * Q15_ONE as i64 / dt) as i32;
        // polyBLEP: 2x - x^2 - 1 (in Q15)
        let x2 = q15_mul(x as i16, x as i16);
        let result = 2 * x - i32::from(x2) - Q15_ONE;
        result.clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16
    } else if t_norm > full - dt {
        // Just before discontinuity
        let x = ((t_norm - full) * Q15_ONE as i64 / dt) as i32;
        let x2 = q15_mul(x as i16, x as i16);
        let result = 2 * x + i32::from(x2) + Q15_ONE;
        result.clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// ADSR Envelope
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnvStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// ADSR envelope generator outputting Q15 values (0 = silent, Q15_MAX = full).
struct Envelope {
    stage: EnvStage,
    /// Current level in Q15 (0..Q15_MAX).
    level: i32,
    /// Per-sample increment for attack phase (positive, Q15).
    attack_rate: i32,
    /// Per-sample decrement for decay phase (positive, Q15).
    decay_rate: i32,
    /// Sustain level in Q15.
    sustain_level: i32,
    /// Per-sample decrement for release phase (positive, Q15).
    release_rate: i32,
}

impl Envelope {
    const fn new() -> Self {
        Self {
            stage: EnvStage::Idle,
            level: 0,
            attack_rate: 0,
            decay_rate: 0,
            sustain_level: 0,
            release_rate: 0,
        }
    }

    /// Configure envelope times. Parameters are in milliseconds.
    fn configure(&mut self, attack_ms: u16, decay_ms: u16, sustain_pct: u8, release_ms: u16) {
        // rate = Q15_MAX / (time_ms * SAMPLE_RATE / 1000)
        // = 32767 * 1000 / (time_ms * 22050)
        // = 32_767_000 / (time_ms * 22050)
        let q15_max = Q15_ONE - 1;
        let sr = SAMPLE_RATE as i32;

        self.attack_rate = if attack_ms == 0 {
            q15_max
        } else {
            (q15_max * 1000 / (i32::from(attack_ms) * sr / 1000)).max(1)
        };

        self.decay_rate = if decay_ms == 0 {
            q15_max
        } else {
            (q15_max * 1000 / (i32::from(decay_ms) * sr / 1000)).max(1)
        };

        self.sustain_level = q15_max * i32::from(sustain_pct) / 100;

        self.release_rate = if release_ms == 0 {
            q15_max
        } else {
            (q15_max * 1000 / (i32::from(release_ms) * sr / 1000)).max(1)
        };
    }

    fn gate_on(&mut self) {
        self.stage = EnvStage::Attack;
    }

    fn gate_off(&mut self) {
        if self.stage != EnvStage::Idle {
            self.stage = EnvStage::Release;
        }
    }

    fn is_active(&self) -> bool {
        self.stage != EnvStage::Idle
    }

    /// Advance one sample, returning the envelope level in Q15 (0..32767).
    fn tick(&mut self) -> i16 {
        match self.stage {
            EnvStage::Idle => 0,
            EnvStage::Attack => {
                self.level += self.attack_rate;
                if self.level >= Q15_ONE - 1 {
                    self.level = Q15_ONE - 1;
                    self.stage = EnvStage::Decay;
                }
                self.level as i16
            }
            EnvStage::Decay => {
                self.level -= self.decay_rate;
                if self.level <= self.sustain_level {
                    self.level = self.sustain_level;
                    self.stage = EnvStage::Sustain;
                }
                self.level as i16
            }
            EnvStage::Sustain => self.level as i16,
            EnvStage::Release => {
                self.level -= self.release_rate;
                if self.level <= 0 {
                    self.level = 0;
                    self.stage = EnvStage::Idle;
                }
                self.level as i16
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Moog Ladder Filter (Huovilainen model)
// ---------------------------------------------------------------------------

/// 4-pole Moog ladder filter using the Huovilainen model with per-stage
/// tanh nonlinearity for authentic Moog saturation character.
///
/// The filter processes at 2x oversampling internally to improve accuracy
/// near the Nyquist frequency.
struct LadderFilter {
    /// Four single-pole filter stages (state variables), in Q16.
    stage: [i32; 4],
    /// Delayed stage outputs for the feedback path, in Q16.
    stage_tanh: [i32; 4],
    /// Cutoff coefficient (0..Q16_ONE), derived from cutoff frequency.
    cutoff_coeff: i32,
    /// Resonance amount (0..Q16_ONE). At Q16_ONE, filter self-oscillates.
    resonance: i32,
}

impl LadderFilter {
    const fn new() -> Self {
        Self {
            stage: [0; 4],
            stage_tanh: [0; 4],
            cutoff_coeff: 0,
            resonance: 0,
        }
    }

    /// Set the cutoff frequency as a MIDI-style value (0-127).
    /// Maps 0 -> ~20Hz, 127 -> ~20kHz (roughly) with an exponential curve.
    fn set_cutoff(&mut self, cutoff_midi: u8) {
        // Map 0-127 to a filter coefficient.
        // We want an exponential mapping for musical response.
        // cutoff_freq = 20 * 2^(cutoff_midi * 10 / 127) Hz
        // But we need the coefficient: g = 1 - exp(-2 * pi * fc / fs)
        // For the oversampled rate (2x): fs_os = 2 * SAMPLE_RATE
        //
        // Simplified: use a precomputed polynomial approximation.
        // For cutoff_midi 0..127, map to coefficient 0..~Q16_ONE
        // Using a rough exponential curve.
        let c = i32::from(cutoff_midi);

        // Attempt a piecewise-linear approximation of the exponential mapping
        // that maps 0 -> very small, 64 -> moderate, 127 -> near 1.0
        // coeff = (c/127)^3 gives a nice exponential feel
        // In Q16: (c * Q16_ONE / 127)^3 / Q16_ONE^2
        let norm = c * Q16_ONE / 127; // Q16, 0..Q16_ONE
        let sq = q16_mul(norm, norm);
        let cube = q16_mul(sq, norm);

        // Scale so max value reaches about 0.9 * Q16_ONE (don't go fully open)
        self.cutoff_coeff = q16_mul(cube, Q16_ONE * 9 / 10);
    }

    /// Set resonance (0-127 MIDI-style). 127 = self-oscillation.
    fn set_resonance(&mut self, resonance_midi: u8) {
        // Map 0-127 to 0..4.0 in Q16 (4.0 is where Moog filters self-oscillate)
        let r = i32::from(resonance_midi);
        self.resonance = r * 4 * Q16_ONE / 127;
    }

    /// Maximum absolute value for filter stage accumulators (±4.0 in Q16).
    /// This prevents runaway integration even with extreme resonance settings.
    const STAGE_CLAMP: i32 = 4 * Q16_ONE;

    /// Process one sample through the 4-pole ladder filter.
    /// Input and output are in Q15.
    fn process(&mut self, input: i16) -> i16 {
        // Huovilainen model with 2x oversampling
        let input_q16 = i32::from(input) << 1; // Q15 -> Q16

        // Two iterations for 2x oversampling
        for _ in 0..2 {
            // Feedback: input - resonance * stage4_output
            // The tanh on the feedback path gives Moog-like saturation
            let feedback = q16_mul(self.resonance, self.stage_tanh[3]);
            let u = tanh_q16(input_q16 - feedback);

            // Four cascaded one-pole sections with per-stage nonlinearity
            // Each stage: y[n] = y[n-1] + g * (tanh(input) - tanh(y[n-1]))
            let g = self.cutoff_coeff;

            // Stage 0
            let diff0 = tanh_q16(u) - self.stage_tanh[0];
            self.stage[0] =
                (self.stage[0] + q16_mul(g, diff0)).clamp(-Self::STAGE_CLAMP, Self::STAGE_CLAMP);
            self.stage_tanh[0] = tanh_q16(self.stage[0]);

            // Stage 1
            let diff1 = self.stage_tanh[0] - self.stage_tanh[1];
            self.stage[1] =
                (self.stage[1] + q16_mul(g, diff1)).clamp(-Self::STAGE_CLAMP, Self::STAGE_CLAMP);
            self.stage_tanh[1] = tanh_q16(self.stage[1]);

            // Stage 2
            let diff2 = self.stage_tanh[1] - self.stage_tanh[2];
            self.stage[2] =
                (self.stage[2] + q16_mul(g, diff2)).clamp(-Self::STAGE_CLAMP, Self::STAGE_CLAMP);
            self.stage_tanh[2] = tanh_q16(self.stage[2]);

            // Stage 3
            let diff3 = self.stage_tanh[2] - self.stage_tanh[3];
            self.stage[3] =
                (self.stage[3] + q16_mul(g, diff3)).clamp(-Self::STAGE_CLAMP, Self::STAGE_CLAMP);
            self.stage_tanh[3] = tanh_q16(self.stage[3]);
        }

        // Output from stage 3, convert Q16 back to Q15
        let out = self.stage_tanh[3] >> 1;
        out.clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16
    }

    /// Reset filter state (avoid clicks when starting a new note).
    fn reset(&mut self) {
        self.stage = [0; 4];
        self.stage_tanh = [0; 4];
    }
}

// ---------------------------------------------------------------------------
// Voice
// ---------------------------------------------------------------------------

/// A single monophonic voice with two oscillators, envelopes, and a filter.
struct Voice {
    osc1: Oscillator,
    osc2: Oscillator,
    amp_env: Envelope,
    filter_env: Envelope,
    filter: LadderFilter,
    note: u8,
    active: bool,
    /// Velocity gain in Q15 (0..Q15_MAX). Scales the amplitude envelope
    /// so softer hits produce quieter sounds.
    velocity_gain: i16,
}

impl Voice {
    const fn new() -> Self {
        Self {
            osc1: Oscillator::new(),
            osc2: Oscillator::new(),
            amp_env: Envelope::new(),
            filter_env: Envelope::new(),
            filter: LadderFilter::new(),
            note: 0,
            active: false,
            velocity_gain: Q15_MAX,
        }
    }
}

// ---------------------------------------------------------------------------
// Synth Engine (public API)
// ---------------------------------------------------------------------------

/// The complete synthesizer state.
pub struct SynthEngine {
    voice: Voice,
    /// Cached config parameters to detect changes.
    osc1_waveform: Waveform,
    osc2_waveform: Waveform,
    osc2_detune: i16,
    osc2_semitone: i8,
    osc_mix: i16,
    filter_cutoff: u8,
    filter_resonance: u8,
    filter_env_amount: i16,
    master_volume: i16,
    /// Stereo reverb processor (Freeverb algorithm).
    reverb: Freeverb,
}

impl SynthEngine {
    pub const fn new() -> Self {
        Self {
            voice: Voice::new(),
            osc1_waveform: Waveform::Saw,
            osc2_waveform: Waveform::Saw,
            osc2_detune: 7,
            osc2_semitone: 0,
            osc_mix: Q15_MAX / 2,
            filter_cutoff: 80,
            filter_resonance: 40,
            filter_env_amount: Q15_MAX / 2,
            master_volume: Q15_MAX / 2,
            reverb: Freeverb::new(),
        }
    }

    /// Apply configuration from the serializable config struct.
    pub fn apply_config(&mut self, cfg: &SynthConfig) {
        self.osc1_waveform = Waveform::from_u8(cfg.osc1_waveform);
        self.osc2_waveform = Waveform::from_u8(cfg.osc2_waveform);
        self.osc2_detune = i16::from(cfg.osc2_detune_cents);
        self.osc2_semitone = cfg.osc2_semitone;
        // Use i32 intermediate to avoid i16 overflow for values near 127
        self.osc_mix =
            (i32::from(cfg.osc_mix) * i32::from(Q15_MAX) / 127).clamp(0, i32::from(Q15_MAX)) as i16;
        self.filter_cutoff = cfg.filter_cutoff;
        self.filter_resonance = cfg.filter_resonance;
        self.filter_env_amount = (i32::from(cfg.filter_env_amount) * i32::from(Q15_MAX) / 127)
            .clamp(0, i32::from(Q15_MAX)) as i16;
        self.master_volume = (i32::from(cfg.master_volume) * i32::from(Q15_MAX) / 127)
            .clamp(0, i32::from(Q15_MAX)) as i16;

        self.voice.filter.set_cutoff(cfg.filter_cutoff);
        self.voice.filter.set_resonance(cfg.filter_resonance);

        self.voice.amp_env.configure(
            cfg.amp_attack_ms,
            cfg.amp_decay_ms,
            cfg.amp_sustain_pct,
            cfg.amp_release_ms,
        );
        self.voice.filter_env.configure(
            cfg.filter_attack_ms,
            cfg.filter_decay_ms,
            cfg.filter_sustain_pct,
            cfg.filter_release_ms,
        );

        self.reverb
            .set_params(cfg.reverb_size, cfg.reverb_damping, cfg.reverb_mix);
    }

    /// Trigger a note-on event.
    pub fn note_on(&mut self, note: u8, velocity: u8) {
        let v = &mut self.voice;

        v.note = note;
        v.active = true;

        // Set oscillator frequencies
        v.osc1.set_frequency(note, 0);
        let osc2_note = (i16::from(note) + i16::from(self.osc2_semitone)).clamp(0, 127) as u8;
        v.osc2.set_frequency(osc2_note, self.osc2_detune);

        // Reset filter to avoid artifacts from previous note
        v.filter.reset();

        // Trigger envelopes
        v.amp_env.gate_on();
        v.filter_env.gate_on();

        // Scale amplitude by velocity (0-127 -> Q15 gain)
        v.velocity_gain = (i32::from(velocity.clamp(1, 127)) * i32::from(Q15_MAX) / 127) as i16;
    }

    /// Trigger a note-off event.
    pub fn note_off(&mut self, note: u8) {
        let v = &mut self.voice;
        if v.note == note && v.active {
            v.amp_env.gate_off();
            v.filter_env.gate_off();
        }
    }

    /// Generate one audio sample as a Q15 signed value (-32768..32767).
    ///
    /// This is the core sample generation method. Use [`tick`] for 8-bit PWM
    /// output or call this directly for 16-bit USB audio output.
    pub fn tick_i16(&mut self) -> i16 {
        let v = &mut self.voice;

        if !v.active && !v.amp_env.is_active() {
            return 0; // Silence
        }

        // Update oscillator drift periodically
        v.osc1.update_drift();
        v.osc2.update_drift();

        // Generate oscillator samples
        let osc1_out = v.osc1.tick(self.osc1_waveform);
        let osc2_out = v.osc2.tick(self.osc2_waveform);

        // Mix oscillators: mix is in Q15 (0 = all osc1, Q15_MAX = all osc2)
        let inv_mix = Q15_MAX - self.osc_mix;
        let mixed = (i32::from(q15_mul(osc1_out, inv_mix))
            + i32::from(q15_mul(osc2_out, self.osc_mix)))
        .clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16;

        // Apply filter envelope to cutoff
        let filter_env_val = v.filter_env.tick();
        let env_offset = q15_mul(filter_env_val, self.filter_env_amount);
        // Modulate cutoff: base + envelope contribution
        let modulated_cutoff = (i32::from(self.filter_cutoff)
            + (i32::from(env_offset) * 127 / Q15_ONE))
            .clamp(0, 127) as u8;
        v.filter.set_cutoff(modulated_cutoff);

        // Run through the ladder filter
        let filtered = v.filter.process(mixed);

        // Apply amplitude envelope
        let amp_env_val = v.amp_env.tick();
        let shaped = q15_mul(filtered, amp_env_val);

        // Apply velocity scaling
        let velocity_scaled = q15_mul(shaped, v.velocity_gain);

        // Apply master volume
        let final_sample = q15_mul(velocity_scaled, self.master_volume);

        // Check if voice has finished releasing
        if !v.amp_env.is_active() {
            v.active = false;
        }

        final_sample
    }

    /// Generate one stereo audio sample pair as Q15 signed values.
    ///
    /// The mono synth output is processed through the Freeverb stereo
    /// reverb, producing a decorrelated left/right pair suitable for
    /// USB stereo audio output.
    pub fn tick_stereo(&mut self) -> (i16, i16) {
        let mono = self.tick_i16();
        self.reverb.process(mono)
    }

    /// Generate one audio sample, returned as a u8 (0-255) suitable for PWM output.
    /// NOTE: Returns dry mono output without reverb processing.
    /// For reverb-processed stereo output, use [`tick_stereo()`].
    #[allow(dead_code)]
    pub fn tick(&mut self) -> u8 {
        let sample = self.tick_i16();

        // Convert Q15 signed (-32768..32767) to unsigned 8-bit (0..255) for PWM
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let pwm_val = ((i32::from(sample) + Q15_ONE) >> 8) as u8;
        pwm_val
    }
}
