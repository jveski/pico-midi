//! PWM audio output driver for the synthesizer.
//!
//! Uses a PWM peripheral at high frequency (~490 kHz on RP2040) as a crude DAC,
//! with the duty cycle modulated at the audio sample rate (22,050 Hz) via a
//! ticker-driven loop. An external RC low-pass filter on the PWM pin smooths
//! the output into an analog audio signal suitable for driving a small speaker
//! or amplifier.
//!
//! Recommended output circuit:
//! ```text
//!   PWM pin ---[68 Ω]---+--- audio out
//!                        |
//!                      [100 nF]
//!                        |
//!                       GND
//! ```
//! For better quality, use a second-stage RC filter (see RP2040 HW design guide).

use core::cell::RefCell;

use embassy_rp::pwm::{Config as PwmConfig, Pwm};
use embassy_rp::Peri;
use embassy_time::Ticker;

use crate::synth::{SynthEngine, SAMPLE_RATE};

/// PWM top value. 255 gives ~490 kHz carrier frequency on RP2040 (125 MHz / 256)
/// and ~586 kHz on RP2350 (150 MHz / 256). Both are well above audible range.
const PWM_TOP: u16 = 255;

/// Run the audio synthesis loop. This drives a PWM output pin at the audio
/// sample rate, generating samples from the synth engine in real time.
///
/// The synth engine is shared via `RefCell` so the MIDI polling task can
/// trigger note-on/off events while this task reads samples. Both tasks
/// run cooperatively on the same single-threaded executor, so borrows
/// never overlap.
///
/// This function never returns.
pub async fn run<'d, T: embassy_rp::pwm::Slice>(
    synth: &RefCell<SynthEngine>,
    pwm_slice: Peri<'d, T>,
    pwm_pin: Peri<'d, impl embassy_rp::pwm::ChannelAPin<T>>,
) -> ! {
    // Configure PWM for audio output
    let mut config = PwmConfig::default();
    config.top = PWM_TOP;
    config.compare_a = PWM_TOP / 2; // Start at midpoint (silence)

    let mut pwm = Pwm::new_output_a(pwm_slice, pwm_pin, config.clone());

    // Ticker fires at the audio sample rate
    let mut ticker = Ticker::every(embassy_time::Duration::from_hz(u64::from(SAMPLE_RATE)));

    loop {
        ticker.next().await;

        // Generate one audio sample from the synth engine.
        // The borrow is released before the next await point, ensuring
        // no overlap with the polling task's borrows.
        let sample = synth.borrow_mut().tick();

        // Update PWM duty cycle with the new sample
        config.compare_a = u16::from(sample);
        pwm.set_config(&config);
    }
}
