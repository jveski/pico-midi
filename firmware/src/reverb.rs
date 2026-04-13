//! Freeverb — stereo algorithmic reverb using Schroeder-Moorer topology.
//!
//! Implements the classic Freeverb algorithm (Jezar at Dreampoint) with:
//! - 8 parallel lowpass-feedback comb filters (LBCF) per channel
//! - 4 series allpass diffusers per channel
//! - Stereo decorrelation via offset delay lengths (stereospread = 23)
//!
//! All DSP uses Q15 fixed-point (i16) for delay-line storage and Q16 (i32)
//! for intermediate calculations, keeping everything efficient on the
//! RP2350 Cortex-M33 without FPU.
//!
//! The algorithm supports very long reverb tails (10+ seconds) without
//! graininess thanks to:
//! - 8 mutually-prime comb filter lengths creating dense echo patterns
//! - Lowpass filtering in the comb feedback path for natural HF decay
//! - 4 allpass diffusers that smear remaining periodicity
//!
//! References:
//! - J. O. Smith III, "Physical Audio Signal Processing", §3.8 Freeverb
//! - Jezar's original Freeverb (public domain, June 2000)

use crate::synth::SAMPLE_RATE;

// ---------------------------------------------------------------------------
// Fixed-point helpers (local to avoid coupling with synth module internals)
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
// Freeverb tuning constants
// ---------------------------------------------------------------------------

// Original Freeverb delay lengths (at 44100 Hz). We scale them for our
// sample rate to preserve the same reverb character.
//
// Comb filter delay lengths (samples at 44100 Hz):
const COMB_LENGTHS_44100: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];

// Allpass filter delay lengths (samples at 44100 Hz):
const ALLPASS_LENGTHS_44100: [usize; 4] = [556, 441, 341, 225];

// Stereospread: offset added to right-channel delay lengths.
const STEREO_SPREAD: usize = 23;

/// Scale a delay length from 44100 Hz to our actual sample rate.
/// Uses integer arithmetic: len * SAMPLE_RATE / 44100, rounded up.
const fn scale_len(len_44100: usize) -> usize {
    // We need (len * SAMPLE_RATE + 44099) / 44100 for ceiling division.
    // At 22050 Hz this gives exactly len/2 for even lengths.
    (len_44100 * SAMPLE_RATE as usize).div_ceil(44100)
}

// Scaled comb delay lengths for left channel.
const COMB_L0: usize = scale_len(COMB_LENGTHS_44100[0]);
const COMB_L1: usize = scale_len(COMB_LENGTHS_44100[1]);
const COMB_L2: usize = scale_len(COMB_LENGTHS_44100[2]);
const COMB_L3: usize = scale_len(COMB_LENGTHS_44100[3]);
const COMB_L4: usize = scale_len(COMB_LENGTHS_44100[4]);
const COMB_L5: usize = scale_len(COMB_LENGTHS_44100[5]);
const COMB_L6: usize = scale_len(COMB_LENGTHS_44100[6]);
const COMB_L7: usize = scale_len(COMB_LENGTHS_44100[7]);

// Scaled comb delay lengths for right channel (+ stereospread before scaling).
const COMB_R0: usize = scale_len(COMB_LENGTHS_44100[0] + STEREO_SPREAD);
const COMB_R1: usize = scale_len(COMB_LENGTHS_44100[1] + STEREO_SPREAD);
const COMB_R2: usize = scale_len(COMB_LENGTHS_44100[2] + STEREO_SPREAD);
const COMB_R3: usize = scale_len(COMB_LENGTHS_44100[3] + STEREO_SPREAD);
const COMB_R4: usize = scale_len(COMB_LENGTHS_44100[4] + STEREO_SPREAD);
const COMB_R5: usize = scale_len(COMB_LENGTHS_44100[5] + STEREO_SPREAD);
const COMB_R6: usize = scale_len(COMB_LENGTHS_44100[6] + STEREO_SPREAD);
const COMB_R7: usize = scale_len(COMB_LENGTHS_44100[7] + STEREO_SPREAD);

// Scaled allpass delay lengths for left channel.
const AP_L0: usize = scale_len(ALLPASS_LENGTHS_44100[0]);
const AP_L1: usize = scale_len(ALLPASS_LENGTHS_44100[1]);
const AP_L2: usize = scale_len(ALLPASS_LENGTHS_44100[2]);
const AP_L3: usize = scale_len(ALLPASS_LENGTHS_44100[3]);

// Scaled allpass delay lengths for right channel.
const AP_R0: usize = scale_len(ALLPASS_LENGTHS_44100[0] + STEREO_SPREAD);
const AP_R1: usize = scale_len(ALLPASS_LENGTHS_44100[1] + STEREO_SPREAD);
const AP_R2: usize = scale_len(ALLPASS_LENGTHS_44100[2] + STEREO_SPREAD);
const AP_R3: usize = scale_len(ALLPASS_LENGTHS_44100[3] + STEREO_SPREAD);

// Freeverb allpass feedback coefficient: 0.5 in Q15 = 16384
const ALLPASS_FEEDBACK: i16 = (Q15_ONE / 2) as i16;

// Scaling constants from Freeverb source (tuning.h):
// roomsize maps [0.0, 1.0] -> [offsetroom, offsetroom + scaleroom]
// where scaleroom = 0.28, offsetroom = 0.7
// So feedback = roomsize_param * 0.28 + 0.7, range [0.7, 0.98]
//
// damping maps [0.0, 1.0] -> [0.0, scaledamp] where scaledamp = 0.4
// So damp_coeff = damp_param * 0.4, range [0.0, 0.4]

// ---------------------------------------------------------------------------
// Lowpass-Feedback Comb Filter (LBCF)
// ---------------------------------------------------------------------------

/// A single lowpass-feedback comb filter as used in Freeverb.
///
/// Transfer function: z^{-N} / (1 - f * H_lp(z) * z^{-N})
/// where H_lp(z) = (1-d) / (1 - d*z^{-1}) is a one-pole lowpass.
struct LbcfComb<const N: usize> {
    buf: [i16; N],
    pos: usize,
    /// One-pole lowpass filter state (Q15).
    filter_store: i32,
}

impl<const N: usize> LbcfComb<N> {
    const fn new() -> Self {
        Self {
            buf: [0i16; N],
            pos: 0,
            filter_store: 0,
        }
    }

    /// Process one sample through the comb filter.
    ///
    /// - `input`: input sample in Q15
    /// - `feedback`: feedback coefficient in Q15 (controls decay time)
    /// - `damp1`: damping coefficient (1 - d) in Q15
    /// - `damp2`: damping coefficient (d) in Q15
    ///
    /// Returns the output sample in Q15.
    #[inline]
    fn process(&mut self, input: i16, feedback: i16, damp1: i16, damp2: i16) -> i16 {
        let output = self.buf[self.pos];

        // One-pole lowpass: y[n] = (1-d) * x[n] + d * y[n-1]
        // In Q15: filter_store = damp1 * output + damp2 * filter_store
        self.filter_store =
            i32::from(q15_mul(output, damp1)) + i32::from(q15_mul(self.filter_store as i16, damp2));
        self.filter_store = self
            .filter_store
            .clamp(i32::from(Q15_MIN), i32::from(Q15_MAX));

        // Write input + feedback * filtered_output back into delay line
        let fb_sample = q15_mul(self.filter_store as i16, feedback);
        let new_val = i32::from(input) + i32::from(fb_sample);
        self.buf[self.pos] = new_val.clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16;

        self.pos += 1;
        if self.pos >= N {
            self.pos = 0;
        }

        output
    }

    #[allow(dead_code)]
    fn clear(&mut self) {
        self.buf.fill(0);
        self.pos = 0;
        self.filter_store = 0;
    }
}

// ---------------------------------------------------------------------------
// Allpass diffuser
// ---------------------------------------------------------------------------

/// Schroeder allpass filter used for diffusion.
///
/// Transfer function: (-g + z^{-N}) / (1 - g * z^{-N})
struct AllpassFilter<const N: usize> {
    buf: [i16; N],
    pos: usize,
}

impl<const N: usize> AllpassFilter<N> {
    const fn new() -> Self {
        Self {
            buf: [0i16; N],
            pos: 0,
        }
    }

    /// Process one sample through the allpass.
    #[inline]
    fn process(&mut self, input: i16) -> i16 {
        let buf_out = self.buf[self.pos];

        // output = -g * input + buf_out
        // buf[pos] = input + g * buf_out
        let neg_g_input = q15_mul(input, ALLPASS_FEEDBACK);
        let output = (i32::from(buf_out) - i32::from(neg_g_input))
            .clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16;

        let g_bufout = q15_mul(buf_out, ALLPASS_FEEDBACK);
        let new_buf =
            (i32::from(input) + i32::from(g_bufout)).clamp(i32::from(Q15_MIN), i32::from(Q15_MAX));
        self.buf[self.pos] = new_buf as i16;

        self.pos += 1;
        if self.pos >= N {
            self.pos = 0;
        }

        output
    }

    #[allow(dead_code)]
    fn clear(&mut self) {
        self.buf.fill(0);
        self.pos = 0;
    }
}

// ---------------------------------------------------------------------------
// Freeverb stereo processor
// ---------------------------------------------------------------------------

/// Complete Freeverb stereo reverb processor.
///
/// Accepts a mono input and produces a stereo output pair.
/// The left and right channels use identical topologies but with
/// slightly different delay lengths for stereo decorrelation.
pub struct Freeverb {
    // Left channel: 8 LBCF combs + 4 allpasses
    comb_l: (
        LbcfComb<COMB_L0>,
        LbcfComb<COMB_L1>,
        LbcfComb<COMB_L2>,
        LbcfComb<COMB_L3>,
        LbcfComb<COMB_L4>,
        LbcfComb<COMB_L5>,
        LbcfComb<COMB_L6>,
        LbcfComb<COMB_L7>,
    ),
    ap_l: (
        AllpassFilter<AP_L0>,
        AllpassFilter<AP_L1>,
        AllpassFilter<AP_L2>,
        AllpassFilter<AP_L3>,
    ),

    // Right channel: 8 LBCF combs + 4 allpasses
    comb_r: (
        LbcfComb<COMB_R0>,
        LbcfComb<COMB_R1>,
        LbcfComb<COMB_R2>,
        LbcfComb<COMB_R3>,
        LbcfComb<COMB_R4>,
        LbcfComb<COMB_R5>,
        LbcfComb<COMB_R6>,
        LbcfComb<COMB_R7>,
    ),
    ap_r: (
        AllpassFilter<AP_R0>,
        AllpassFilter<AP_R1>,
        AllpassFilter<AP_R2>,
        AllpassFilter<AP_R3>,
    ),

    /// Feedback coefficient for comb filters, Q15. Controls reverb time.
    feedback: i16,
    /// Lowpass coefficient: (1 - damp) in Q15. Higher = brighter reverb.
    damp1: i16,
    /// Lowpass coefficient: damp in Q15. Higher = darker reverb.
    damp2: i16,
    /// Wet mix level, Q15. 0 = fully dry, Q15_MAX = fully wet.
    wet: i16,
    /// Dry mix level, Q15. Complement of wet.
    dry: i16,
}

impl Freeverb {
    pub const fn new() -> Self {
        Self {
            comb_l: (
                LbcfComb::new(),
                LbcfComb::new(),
                LbcfComb::new(),
                LbcfComb::new(),
                LbcfComb::new(),
                LbcfComb::new(),
                LbcfComb::new(),
                LbcfComb::new(),
            ),
            ap_l: (
                AllpassFilter::new(),
                AllpassFilter::new(),
                AllpassFilter::new(),
                AllpassFilter::new(),
            ),
            comb_r: (
                LbcfComb::new(),
                LbcfComb::new(),
                LbcfComb::new(),
                LbcfComb::new(),
                LbcfComb::new(),
                LbcfComb::new(),
                LbcfComb::new(),
                LbcfComb::new(),
            ),
            ap_r: (
                AllpassFilter::new(),
                AllpassFilter::new(),
                AllpassFilter::new(),
                AllpassFilter::new(),
            ),
            // Default: no reverb tail (call set_params to activate).
            // apply_config() always calls set_params() before any audio is generated.
            feedback: 0,
            damp1: Q15_MAX,
            damp2: 0,
            wet: (Q15_ONE / 2) as i16,
            dry: (Q15_ONE / 2) as i16,
        }
    }

    /// Set reverb parameters from 0-127 MIDI-style values.
    ///
    /// - `room_size`: 0-127. Maps to feedback coefficient [0.7, 0.98].
    ///   Higher values = longer reverb tail.
    /// - `damping`: 0-127. Maps to lowpass damping [0.0, 0.4].
    ///   Higher values = darker, more absorbed high frequencies.
    /// - `wet_mix`: 0-127. Dry/wet balance. 0 = fully dry, 127 = fully wet.
    pub fn set_params(&mut self, room_size: u8, damping: u8, wet_mix: u8) {
        // feedback = room_size/127 * 0.28 + 0.7 (range [0.7, 0.98])
        // In Q15: 0.7 * 32767 = 22937, 0.28 * 32767 = 9175
        let room_norm = i32::from(room_size); // 0..127
        self.feedback = (22937 + room_norm * 9175 / 127).clamp(0, i32::from(Q15_MAX)) as i16;

        // damp = damping/127 * 0.4 (range [0.0, 0.4])
        // In Q15: 0.4 * 32767 = 13107
        let damp_norm = i32::from(damping);
        let damp_q15 = (damp_norm * 13107 / 127).clamp(0, i32::from(Q15_MAX)) as i16;
        self.damp1 = (i32::from(Q15_MAX) - i32::from(damp_q15)).clamp(0, i32::from(Q15_MAX)) as i16;
        self.damp2 = damp_q15;

        // Wet/dry mix
        let wet_norm = i32::from(wet_mix);
        self.wet = (wet_norm * i32::from(Q15_MAX) / 127).clamp(0, i32::from(Q15_MAX)) as i16;
        self.dry = (i32::from(Q15_MAX) - i32::from(self.wet)).max(0) as i16;
    }

    /// Process one mono input sample, returning a stereo (left, right) pair.
    ///
    /// The input is fed into 8 parallel LBCF comb filters whose outputs are
    /// summed, then passed through 4 series allpass diffusers. This is done
    /// independently for left and right channels with offset delay lengths.
    pub fn process(&mut self, input: i16) -> (i16, i16) {
        // Scale input down to avoid clipping in the comb filter sum.
        // Freeverb scales by a "fixed gain" of 0.015; we use a shift
        // which effectively divides by 8 (the number of combs) and then
        // some extra headroom. Dividing by 8 is a right shift by 3.
        let scaled_input = input >> 3;

        let fb = self.feedback;
        let d1 = self.damp1;
        let d2 = self.damp2;

        // Left channel combs (parallel sum)
        let sum_l = i32::from(self.comb_l.0.process(scaled_input, fb, d1, d2))
            + i32::from(self.comb_l.1.process(scaled_input, fb, d1, d2))
            + i32::from(self.comb_l.2.process(scaled_input, fb, d1, d2))
            + i32::from(self.comb_l.3.process(scaled_input, fb, d1, d2))
            + i32::from(self.comb_l.4.process(scaled_input, fb, d1, d2))
            + i32::from(self.comb_l.5.process(scaled_input, fb, d1, d2))
            + i32::from(self.comb_l.6.process(scaled_input, fb, d1, d2))
            + i32::from(self.comb_l.7.process(scaled_input, fb, d1, d2));
        let mut out_l = sum_l.clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16;

        // Right channel combs (parallel sum)
        let sum_r = i32::from(self.comb_r.0.process(scaled_input, fb, d1, d2))
            + i32::from(self.comb_r.1.process(scaled_input, fb, d1, d2))
            + i32::from(self.comb_r.2.process(scaled_input, fb, d1, d2))
            + i32::from(self.comb_r.3.process(scaled_input, fb, d1, d2))
            + i32::from(self.comb_r.4.process(scaled_input, fb, d1, d2))
            + i32::from(self.comb_r.5.process(scaled_input, fb, d1, d2))
            + i32::from(self.comb_r.6.process(scaled_input, fb, d1, d2))
            + i32::from(self.comb_r.7.process(scaled_input, fb, d1, d2));
        let mut out_r = sum_r.clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16;

        // Left channel allpasses (series)
        out_l = self.ap_l.0.process(out_l);
        out_l = self.ap_l.1.process(out_l);
        out_l = self.ap_l.2.process(out_l);
        out_l = self.ap_l.3.process(out_l);

        // Right channel allpasses (series)
        out_r = self.ap_r.0.process(out_r);
        out_r = self.ap_r.1.process(out_r);
        out_r = self.ap_r.2.process(out_r);
        out_r = self.ap_r.3.process(out_r);

        // Mix dry + wet
        let left = (i32::from(q15_mul(input, self.dry)) + i32::from(q15_mul(out_l, self.wet)))
            .clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16;
        let right = (i32::from(q15_mul(input, self.dry)) + i32::from(q15_mul(out_r, self.wet)))
            .clamp(i32::from(Q15_MIN), i32::from(Q15_MAX)) as i16;

        (left, right)
    }

    /// Clear all delay lines and filter states. Use when switching
    /// reverb on/off to avoid stale audio bleeding through.
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.comb_l.0.clear();
        self.comb_l.1.clear();
        self.comb_l.2.clear();
        self.comb_l.3.clear();
        self.comb_l.4.clear();
        self.comb_l.5.clear();
        self.comb_l.6.clear();
        self.comb_l.7.clear();

        self.ap_l.0.clear();
        self.ap_l.1.clear();
        self.ap_l.2.clear();
        self.ap_l.3.clear();

        self.comb_r.0.clear();
        self.comb_r.1.clear();
        self.comb_r.2.clear();
        self.comb_r.3.clear();
        self.comb_r.4.clear();
        self.comb_r.5.clear();
        self.comb_r.6.clear();
        self.comb_r.7.clear();

        self.ap_r.0.clear();
        self.ap_r.1.clear();
        self.ap_r.2.clear();
        self.ap_r.3.clear();
    }
}
