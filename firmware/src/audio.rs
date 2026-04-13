//! Audio output drivers for the synthesizer.
//!
//! Provides two output paths:
//! 1. **PWM** – a PWM peripheral at high frequency (~490 kHz) acts as a crude
//!    DAC, modulated at the audio sample rate (22,050 Hz). An external RC
//!    low-pass filter smooths the output into an analog signal.
//! 2. **USB Audio** – 16-bit PCM samples are buffered in a lock-free ring
//!    buffer and drained into USB isochronous IN packets by a separate task,
//!    presenting the device as a UAC 1.0 microphone.
//!
//! Recommended PWM output circuit:
//! ```text
//!   PWM pin ---[68 Ω]---+--- audio out
//!                        |
//!                      [100 nF]
//!                        |
//!                       GND
//! ```
//! For better quality, use a second-stage RC filter (see RP2040 HW design guide).

use core::cell::RefCell;
use core::sync::atomic::{AtomicU16, Ordering};

use embassy_rp::pwm::{Config as PwmConfig, Pwm};
use embassy_rp::Peri;
use embassy_time::Ticker;

use crate::synth::{SynthEngine, SAMPLE_RATE};

/// PWM top value. 255 gives ~490 kHz carrier frequency on RP2040 (125 MHz / 256)
/// and ~586 kHz on RP2350 (150 MHz / 256). Both are well above audible range.
const PWM_TOP: u16 = 255;

// ---------------------------------------------------------------------------
// Lock-free SPSC ring buffer for passing i16 samples from the ticker ISR
// context to the USB audio streaming task.
//
// The buffer is sized to hold several USB frames' worth of samples so the
// USB task can drain a full frame even if it runs slightly behind.
// At 22 050 Hz with 1 ms USB frames, one frame is ~22 samples.
// 128 samples ≈ 5.8 ms of buffering, which is generous.
// ---------------------------------------------------------------------------

/// Number of i16 samples the ring buffer can hold. Must be a power of two.
const RING_BUF_LEN: usize = 128;
const RING_BUF_MASK: usize = RING_BUF_LEN - 1;

/// Ring buffer storage. Written by the ticker task, read by the USB task.
/// We use `AtomicU16` for each slot to allow lock-free access between tasks
/// running on a single-threaded executor (no actual contention, but this
/// keeps the borrow checker happy without `unsafe` shared mutable state).
#[allow(clippy::declare_interior_mutable_const)]
static RING_BUF: [AtomicU16; RING_BUF_LEN] = {
    const INIT: AtomicU16 = AtomicU16::new(0);
    [INIT; RING_BUF_LEN]
};

/// Write index (only modified by the ticker/producer task).
static RING_WR: AtomicU16 = AtomicU16::new(0);
/// Read index (only modified by the USB/consumer task).
static RING_RD: AtomicU16 = AtomicU16::new(0);

/// Push one i16 sample into the ring buffer. Drops the sample if full.
#[inline]
fn ring_push(sample: i16) {
    let wr = RING_WR.load(Ordering::Relaxed) as usize;
    let rd = RING_RD.load(Ordering::Acquire) as usize;
    let next = (wr + 1) & RING_BUF_MASK;
    if next == rd {
        return; // full — drop sample
    }
    RING_BUF[wr].store(sample as u16, Ordering::Relaxed);
    RING_WR.store(next as u16, Ordering::Release);
}

/// Pop one i16 sample from the ring buffer. Returns `None` if empty.
#[inline]
fn ring_pop() -> Option<i16> {
    let rd = RING_RD.load(Ordering::Relaxed) as usize;
    let wr = RING_WR.load(Ordering::Acquire) as usize;
    if rd == wr {
        return None; // empty
    }
    let val = RING_BUF[rd].load(Ordering::Relaxed) as i16;
    RING_RD.store(((rd + 1) & RING_BUF_MASK) as u16, Ordering::Release);
    Some(val)
}

/// Number of samples currently available in the ring buffer.
#[inline]
fn ring_len() -> usize {
    let wr = RING_WR.load(Ordering::Acquire) as usize;
    let rd = RING_RD.load(Ordering::Acquire) as usize;
    (wr.wrapping_sub(rd)) & RING_BUF_MASK
}

// ---------------------------------------------------------------------------
// PWM audio output task
// ---------------------------------------------------------------------------

/// Run the audio synthesis loop. This drives a PWM output pin at the audio
/// sample rate, generating samples from the synth engine in real time.
///
/// When `usb_audio_enabled` is true, each sample is also pushed into the
/// ring buffer for the USB audio streaming task to consume.
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
    usb_audio_enabled: bool,
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
        let sample_i16 = synth.borrow_mut().tick_i16();

        // Update PWM duty cycle with the 8-bit sample
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let pwm_val = ((i32::from(sample_i16) + 32768) >> 8) as u16;
        config.compare_a = pwm_val;
        pwm.set_config(&config);

        // Feed the USB audio ring buffer with the full 16-bit sample
        if usb_audio_enabled {
            ring_push(sample_i16);
        }
    }
}

// ---------------------------------------------------------------------------
// USB audio streaming task
// ---------------------------------------------------------------------------

/// Maximum samples per USB frame at 22 050 Hz (1 ms frame period).
/// 22.05 samples/frame — we budget 23 to handle timing jitter.
const MAX_SAMPLES_PER_FRAME: usize = 23;

/// Size of the USB packet buffer: 23 samples * 2 bytes = 46 bytes.
const USB_PACKET_BUF_SIZE: usize = MAX_SAMPLES_PER_FRAME * 2;

/// Run the USB audio streaming loop. Waits for the host to enable the
/// streaming interface, then continuously drains the ring buffer into
/// isochronous IN packets.
///
/// This function never returns.
pub async fn run_usb_audio<'d, D: embassy_usb::driver::Driver<'d>>(
    stream: &mut crate::usb_audio::AudioStream<'d, D>,
) -> ! {
    loop {
        // Wait until the host selects alt-setting 1 (operational)
        stream.wait_connection().await;
        defmt::info!("USB audio: host connected");

        // Stream until disconnected
        loop {
            // Collect available samples into a packet buffer.
            // At 22 050 Hz, we expect ~22 samples per 1 ms USB frame.
            let mut buf = [0u8; USB_PACKET_BUF_SIZE];
            let avail = ring_len().min(MAX_SAMPLES_PER_FRAME);

            if avail == 0 {
                // No samples ready yet — send an empty packet to keep
                // the isochronous schedule alive. The host expects a
                // packet every frame even if it's zero-length.
                match stream.write(&[]).await {
                    Ok(()) => continue,
                    Err(_) => break, // disconnected
                }
            }

            let byte_len = avail * 2;
            for i in 0..avail {
                let sample = ring_pop().unwrap_or(0);
                let le = sample.to_le_bytes();
                buf[i * 2] = le[0];
                buf[i * 2 + 1] = le[1];
            }

            match stream.write(&buf[..byte_len]).await {
                Ok(()) => {}
                Err(_) => break, // disconnected
            }
        }

        defmt::info!("USB audio: host disconnected");
    }
}
