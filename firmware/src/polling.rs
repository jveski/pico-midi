use core::cell::RefCell;

use embassy_futures::select::select;
use embassy_rp::adc;
use embassy_rp::gpio::Output;
use embassy_rp::i2c;
use embassy_rp::peripherals::{I2C1, USB};
use embassy_rp::usb::Driver;
use embassy_time::{Duration, Instant, Timer};
use embassy_usb::class::midi::MidiClass;

use crate::config::{Config, MAX_ANALOG_INPUTS, MAX_DIGITAL_INPUTS};
use crate::expr::{self, ExprInputs};
use crate::input;
use crate::input_state::InputState;

const CIN_NOTE_OFF: u8 = 0x08;
const CIN_NOTE_ON: u8 = 0x09;
const CIN_CC: u8 = 0x0B;

/// Minimum time (ms) a scale expression must produce a new note before
/// the held button re-triggers.  Prevents rapid note spam from ADC noise
/// near quantisation boundaries.
const RETRIGGER_DEBOUNCE_MS: u64 = 50;

/// Tracks the MIDI note currently sounding for a held button or touch pad.
///
/// When the note expression uses `scale()`, the expression is re-evaluated
/// every loop iteration while held.  If the result changes, the new note
/// is stored as `pending` and a debounce timer starts.  Once the pending
/// note has been stable for [`RETRIGGER_DEBOUNCE_MS`], the old note is
/// released and the new one is sent.
#[derive(Clone, Copy)]
struct HeldNote {
    /// The note value currently sounding (note-on has been sent).
    current: u8,
    /// A candidate note that differs from `current`, awaiting debounce.
    pending: Option<u8>,
    /// When `pending` was first observed.
    pending_since: Instant,
}

impl HeldNote {
    const fn new(note: u8) -> Self {
        Self {
            current: note,
            pending: None,
            pending_since: Instant::MIN,
        }
    }
}

/// Collect GPIO pin numbers for active buttons from config.
fn button_gpios(cfg: &Config) -> [u8; MAX_DIGITAL_INPUTS] {
    let mut gpios = [0u8; MAX_DIGITAL_INPUTS];
    for (i, b) in cfg.active_buttons().iter().enumerate() {
        gpios[i] = b.pin;
    }
    gpios
}

/// Collect GPIO pin numbers for active touch pads from config.
fn touch_gpios(cfg: &Config) -> [u8; MAX_DIGITAL_INPUTS] {
    let mut gpios = [0u8; MAX_DIGITAL_INPUTS];
    for (i, t) in cfg.active_touch_pads().iter().enumerate() {
        gpios[i] = t.pin;
    }
    gpios
}

/// Collect threshold percentages for active touch pads from config.
fn touch_thresholds(cfg: &Config) -> [u8; MAX_DIGITAL_INPUTS] {
    let mut thrs = [33u8; MAX_DIGITAL_INPUTS];
    for (i, t) in cfg.active_touch_pads().iter().enumerate() {
        thrs[i] = t.threshold_pct;
    }
    thrs
}

/// Track which pins we've configured so we can detect config changes.
#[derive(Clone, Copy, PartialEq, Eq)]
struct PinSnapshot {
    button_pins: [u8; MAX_DIGITAL_INPUTS],
    num_buttons: u8,
    touch_pins: [u8; MAX_DIGITAL_INPUTS],
    num_touch: u8,
    pot_pins: [u8; MAX_ANALOG_INPUTS],
    num_pots: u8,
    ldr_enabled: bool,
    ldr_pin: u8,
}

impl PinSnapshot {
    fn from_config(cfg: &Config) -> Self {
        let mut bp = [0u8; MAX_DIGITAL_INPUTS];
        for (i, b) in cfg.active_buttons().iter().enumerate() {
            bp[i] = b.pin;
        }
        let mut tp = [0u8; MAX_DIGITAL_INPUTS];
        for (i, t) in cfg.active_touch_pads().iter().enumerate() {
            tp[i] = t.pin;
        }
        let mut pp = [0u8; MAX_ANALOG_INPUTS];
        for (i, p) in cfg.active_pots().iter().enumerate() {
            pp[i] = p.pin;
        }
        Self {
            button_pins: bp,
            num_buttons: cfg.num_buttons,
            touch_pins: tp,
            num_touch: cfg.num_touch_pads,
            pot_pins: pp,
            num_pots: cfg.num_pots,
            ldr_enabled: cfg.ldr_enabled,
            ldr_pin: cfg.ldr.pin,
        }
    }
}

#[inline]
const fn note_on(channel: u8, note: u8, velocity: u8) -> [u8; 4] {
    [
        CIN_NOTE_ON,
        0x90 | (channel & 0x0F),
        note & 0x7F,
        velocity & 0x7F,
    ]
}

#[inline]
const fn note_off(channel: u8, note: u8) -> [u8; 4] {
    [CIN_NOTE_OFF, 0x80 | (channel & 0x0F), note & 0x7F, 0]
}

#[inline]
const fn control_change(channel: u8, cc: u8, value: u8) -> [u8; 4] {
    [CIN_CC, 0xB0 | (channel & 0x0F), cc & 0x7F, value & 0x7F]
}

/// Send a MIDI packet with a timeout to avoid blocking input polling
/// when no MIDI host is actively reading from the device.
async fn send_midi(midi: &mut MidiClass<'static, Driver<'static, USB>>, pkt: &[u8; 4]) {
    const MIDI_TIMEOUT: Duration = Duration::from_millis(5);
    let _ = select(midi.write_packet(pkt), Timer::after(MIDI_TIMEOUT)).await;
}

/// Main input polling and MIDI sending loop.
///
/// Reads buttons, touch pads, pots, LDR, and accelerometer inputs each
/// iteration, evaluates note/velocity/CC expressions, and sends the
/// resulting MIDI messages.  Also blinks the status LED and watches for
/// pin-assignment changes in the shared config.
pub async fn run(
    midi_class: &mut MidiClass<'static, Driver<'static, USB>>,
    led: &mut Output<'static>,
    adc_inst: &mut adc::Adc<'static, adc::Async>,
    i2c1: i2c::I2c<'static, I2C1, i2c::Async>,
    cfg: &RefCell<Config>,
    input_state: &InputState,
) {
    Timer::after(Duration::from_millis(100)).await;

    // --- Dynamic input initialization ---
    let mut buttons = input::Buttons::new();
    let mut touch = input::TouchPads::new();
    let mut pots: [Option<input::SmoothedAnalog<'static>>; MAX_ANALOG_INPUTS] =
        [const { None }; MAX_ANALOG_INPUTS];
    let mut ldr: Option<input::SmoothedAnalog<'static>> = None;

    // Configure inputs from initial config
    let initial_cfg = *cfg.borrow();
    let mut pin_snapshot = PinSnapshot::from_config(&initial_cfg);

    // Safety: we own all the peripheral singletons via `p` and configure
    // each GPIO only once based on validated config (no duplicates).
    unsafe {
        let bg = button_gpios(&initial_cfg);
        buttons.configure(&bg[..initial_cfg.num_buttons as usize]);

        let tg = touch_gpios(&initial_cfg);
        let tt = touch_thresholds(&initial_cfg);
        touch.configure(
            &tg[..initial_cfg.num_touch_pads as usize],
            &tt[..initial_cfg.num_touch_pads as usize],
        );

        for (i, pot_def) in initial_cfg.active_pots().iter().enumerate() {
            if let Some(ch) = input::adc_channel_from_gpio(pot_def.pin) {
                pots[i] = Some(input::SmoothedAnalog::new(ch, 0.2));
            }
        }

        if initial_cfg.ldr_enabled {
            if let Some(ch) = input::adc_channel_from_gpio(initial_cfg.ldr.pin) {
                ldr = Some(input::SmoothedAnalog::new(ch, 0.15));
            }
        }
    }

    // --- Accelerometer: I2C1, SCL=GP3, SDA=GP2 (hardcoded) ---
    let accel_dead_zone = initial_cfg.accel.dead_zone_tenths;
    let accel_smoothing = initial_cfg.accel.smoothing_pct;
    let mut accel = input::Accelerometer::new(i2c1, accel_dead_zone, accel_smoothing).await;

    let mut last_led_toggle = Instant::now();
    let mut led_on = false;

    // Track the note currently sounding for each held button/pad so
    // that (a) note-off always releases the correct note, and (b) when
    // a scale() expression result changes while held the old note is
    // released and the new one is sent after a debounce period.
    let mut active_button_note: [Option<HeldNote>; MAX_DIGITAL_INPUTS] = [None; MAX_DIGITAL_INPUTS];
    let mut active_touch_note: [Option<HeldNote>; MAX_DIGITAL_INPUTS] = [None; MAX_DIGITAL_INPUTS];

    loop {
        // ~1ms poll interval
        Timer::after(Duration::from_millis(1)).await;

        // Snapshot the shared config for this iteration.
        // The borrow is brief and does not span an await point.
        let cur = *cfg.borrow();

        // Check if pin assignments changed — reconfigure if so.
        let new_snapshot = PinSnapshot::from_config(&cur);
        if new_snapshot != pin_snapshot {
            // Release all held notes before reconfiguring
            for (idx, slot) in active_button_note.iter_mut().enumerate() {
                if let Some(held) = slot.take() {
                    send_midi(midi_class, &note_off(cur.midi_channel, held.current)).await;
                    input_state.set_button(idx as u8, false);
                }
            }
            for (idx, slot) in active_touch_note.iter_mut().enumerate() {
                if let Some(held) = slot.take() {
                    send_midi(midi_class, &note_off(cur.midi_channel, held.current)).await;
                    input_state.set_touch(idx as u8, false);
                }
            }

            // Safety: we're reconfiguring pins based on a validated config.
            // The old pin drivers are dropped inside configure(), releasing
            // the GPIO before the new ones are created.
            unsafe {
                let bg = button_gpios(&cur);
                buttons.configure(&bg[..cur.num_buttons as usize]);

                let tg = touch_gpios(&cur);
                let tt = touch_thresholds(&cur);
                touch.configure(
                    &tg[..cur.num_touch_pads as usize],
                    &tt[..cur.num_touch_pads as usize],
                );

                // Reconfigure pots
                for pot in pots.iter_mut() {
                    *pot = None;
                }
                for (i, pot_def) in cur.active_pots().iter().enumerate() {
                    if let Some(ch) = input::adc_channel_from_gpio(pot_def.pin) {
                        pots[i] = Some(input::SmoothedAnalog::new(ch, 0.2));
                    }
                }

                // Reconfigure LDR
                ldr = None;
                if cur.ldr_enabled {
                    if let Some(ch) = input::adc_channel_from_gpio(cur.ldr.pin) {
                        ldr = Some(input::SmoothedAnalog::new(ch, 0.15));
                    }
                }
            }

            pin_snapshot = new_snapshot;
            defmt::info!(
                "pins reconfigured: {}b {}t {}p",
                cur.num_buttons,
                cur.num_touch_pads,
                cur.num_pots
            );
        }

        // Update accelerometer tuning if changed via the configurator.
        accel.update_params(cur.accel.dead_zone_tenths, cur.accel.smoothing_pct);

        // Update touch thresholds if changed via the configurator.
        let thrs = touch_thresholds(&cur);
        touch.update_thresholds(&thrs[..cur.num_touch_pads as usize]);

        // Build expression inputs from current state
        let expr_inputs = ExprInputs {
            pots: input_state.pots_snapshot(),
            ldr: input_state.ldr_value(),
            accel_x: input_state.accel_x_value(),
            accel_y: input_state.accel_y_value(),
        };

        // Poll buttons
        let num_buttons = cur.num_buttons as usize;
        for evt in buttons.poll().into_iter().take(num_buttons).flatten() {
            let idx = evt.index as usize;
            if evt.pressed {
                let def = &cur.buttons[idx];
                let note = expr::eval(
                    &def.note_expr.code,
                    def.note_expr.len,
                    &expr_inputs,
                    def.note,
                );
                let vel = expr::eval(
                    &def.velocity_expr.code,
                    def.velocity_expr.len,
                    &expr_inputs,
                    def.velocity,
                );
                active_button_note[idx] = Some(HeldNote::new(note));
                send_midi(midi_class, &note_on(cur.midi_channel, note, vel)).await;
            } else {
                let note = active_button_note[idx].map_or(cur.buttons[idx].note, |h| h.current);
                active_button_note[idx] = None;
                send_midi(midi_class, &note_off(cur.midi_channel, note)).await;
            };
            input_state.set_button(evt.index, evt.pressed);
        }

        // Re-evaluate scale() expressions for held buttons.
        // If the quantised note changed, debounce and re-trigger.
        let now = Instant::now();
        for (idx, slot) in active_button_note.iter_mut().take(num_buttons).enumerate() {
            let held = match slot {
                Some(h) => h,
                None => continue,
            };
            let def = &cur.buttons[idx];
            if !expr::has_scale(&def.note_expr.code, def.note_expr.len) {
                continue;
            }
            let note = expr::eval(
                &def.note_expr.code,
                def.note_expr.len,
                &expr_inputs,
                def.note,
            );
            if note == held.current {
                // Expression result matches the sounding note — reset any pending.
                held.pending = None;
                continue;
            }
            match held.pending {
                Some(p) if p == note => {
                    // Same pending note — check if debounce elapsed.
                    if now.duration_since(held.pending_since).as_millis() >= RETRIGGER_DEBOUNCE_MS {
                        let vel = expr::eval(
                            &def.velocity_expr.code,
                            def.velocity_expr.len,
                            &expr_inputs,
                            def.velocity,
                        );
                        send_midi(midi_class, &note_off(cur.midi_channel, held.current)).await;
                        send_midi(midi_class, &note_on(cur.midi_channel, note, vel)).await;
                        held.current = note;
                        held.pending = None;
                    }
                }
                _ => {
                    // New candidate — start debounce timer.
                    held.pending = Some(note);
                    held.pending_since = now;
                }
            }
        }

        // Poll touch pads
        let num_touch = cur.num_touch_pads as usize;
        for evt in touch.poll().await.into_iter().take(num_touch).flatten() {
            let idx = evt.index as usize;
            if evt.pressed {
                let def = &cur.touch_pads[idx];
                let note = expr::eval(
                    &def.note_expr.code,
                    def.note_expr.len,
                    &expr_inputs,
                    def.note,
                );
                let vel = expr::eval(
                    &def.velocity_expr.code,
                    def.velocity_expr.len,
                    &expr_inputs,
                    def.velocity,
                );
                active_touch_note[idx] = Some(HeldNote::new(note));
                send_midi(midi_class, &note_on(cur.midi_channel, note, vel)).await;
            } else {
                let note = active_touch_note[idx].map_or(cur.touch_pads[idx].note, |h| h.current);
                active_touch_note[idx] = None;
                send_midi(midi_class, &note_off(cur.midi_channel, note)).await;
            };
            input_state.set_touch(evt.index, evt.pressed);
        }

        // Re-evaluate scale() expressions for held touch pads.
        let now = Instant::now();
        for (idx, slot) in active_touch_note.iter_mut().take(num_touch).enumerate() {
            let held = match slot {
                Some(h) => h,
                None => continue,
            };
            let def = &cur.touch_pads[idx];
            if !expr::has_scale(&def.note_expr.code, def.note_expr.len) {
                continue;
            }
            let note = expr::eval(
                &def.note_expr.code,
                def.note_expr.len,
                &expr_inputs,
                def.note,
            );
            if note == held.current {
                held.pending = None;
                continue;
            }
            match held.pending {
                Some(p) if p == note => {
                    if now.duration_since(held.pending_since).as_millis() >= RETRIGGER_DEBOUNCE_MS {
                        let vel = expr::eval(
                            &def.velocity_expr.code,
                            def.velocity_expr.len,
                            &expr_inputs,
                            def.velocity,
                        );
                        send_midi(midi_class, &note_off(cur.midi_channel, held.current)).await;
                        send_midi(midi_class, &note_on(cur.midi_channel, note, vel)).await;
                        held.current = note;
                        held.pending = None;
                    }
                }
                _ => {
                    held.pending = Some(note);
                    held.pending_since = now;
                }
            }
        }

        // Poll pots
        let num_pots = cur.num_pots as usize;
        for (i, pot_slot) in pots.iter_mut().take(num_pots).enumerate() {
            if let Some(pot) = pot_slot {
                if let Some(v) = pot.poll(adc_inst, 2).await {
                    let pkt = control_change(cur.midi_channel, cur.pots[i].cc, v);
                    send_midi(midi_class, &pkt).await;
                }
                #[allow(clippy::cast_possible_truncation)] // index fits in u8
                input_state.set_pot(i as u8, pot.current_cc());
            }
        }

        // Poll LDR
        if cur.ldr_enabled {
            if let Some(ldr_input) = &mut ldr {
                if let Some(v) = ldr_input.poll(adc_inst, 2).await {
                    let pkt = control_change(cur.midi_channel, cur.ldr.cc, v);
                    send_midi(midi_class, &pkt).await;
                }
                input_state.set_ldr(ldr_input.current_cc());
            }
        }

        // Poll accelerometer
        if cur.accel.enabled && accel.available {
            let r = accel.poll().await;
            if let Some(x) = r.x_cc {
                send_midi(
                    midi_class,
                    &control_change(cur.midi_channel, cur.accel.x_cc, x),
                )
                .await;
            }
            if let Some(y) = r.y_cc {
                send_midi(
                    midi_class,
                    &control_change(cur.midi_channel, cur.accel.y_cc, y),
                )
                .await;
            }
            if r.tapped {
                send_midi(
                    midi_class,
                    &note_on(cur.midi_channel, cur.accel.tap_note, cur.accel.tap_velocity),
                )
                .await;
                send_midi(midi_class, &note_off(cur.midi_channel, cur.accel.tap_note)).await;
                input_state.set_accel_tap();
            }
            input_state.set_accel_x(accel.current_x_cc());
            input_state.set_accel_y(accel.current_y_cc());
        }

        // Blink status LED every 1s
        if Instant::now().duration_since(last_led_toggle).as_millis() >= 1000 {
            last_led_toggle = Instant::now();
            led_on = !led_on;
            if led_on {
                led.set_high();
            } else {
                led.set_low();
            }
        }
    }
}
