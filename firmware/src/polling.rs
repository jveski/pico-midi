use core::cell::RefCell;

use embassy_futures::select::select;
use embassy_rp::adc;
use embassy_rp::gpio::Output;
use embassy_rp::i2c;
use embassy_rp::peripherals::{I2C1, USB};
use embassy_rp::usb::Driver;
use embassy_time::{Duration, Instant, Timer};
use embassy_usb::class::midi::MidiClass;

use crate::config::{Config, NoteInput, MAX_ANALOG_INPUTS, MAX_DIGITAL_INPUTS};
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

    /// Check whether a re-evaluated note expression has changed.
    ///
    /// Returns `true` when the debounce period has elapsed and `current`
    /// has been updated to `new_note` (caller should send note-off/on).
    fn update(&mut self, new_note: u8, now: Instant) -> bool {
        if new_note == self.current {
            self.pending = None;
            return false;
        }
        match self.pending {
            Some(p) if p == new_note => {
                if now.duration_since(self.pending_since).as_millis() >= RETRIGGER_DEBOUNCE_MS {
                    self.current = new_note;
                    self.pending = None;
                    return true;
                }
            }
            _ => {
                self.pending = Some(new_note);
                self.pending_since = now;
            }
        }
        false
    }
}

/// Collect a single `u8` field from each element of `items` into a
/// fixed-size array, filling unused slots with `default`.
fn collect_field<T, const N: usize>(items: &[T], default: u8, f: impl Fn(&T) -> u8) -> [u8; N] {
    let mut arr = [default; N];
    for (i, item) in items.iter().enumerate().take(N) {
        arr[i] = f(item);
    }
    arr
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
        Self {
            button_pins: collect_field(cfg.active_buttons(), 0, |b| b.pin),
            num_buttons: cfg.num_buttons,
            touch_pins: collect_field(cfg.active_touch_pads(), 0, |t| t.pin),
            num_touch: cfg.num_touch_pads,
            pot_pins: collect_field(cfg.active_pots(), 0, |p| p.pin),
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

/// (Re-)configure all input peripherals from `cfg`.
///
/// Clears existing pot/LDR state before creating new drivers.
///
/// # Safety
/// Caller must ensure the GPIO numbers in `cfg` are valid and that no
/// other code is concurrently using the same pins.
unsafe fn configure_inputs(
    cfg: &Config,
    buttons: &mut input::Buttons,
    touch: &mut input::TouchPads,
    pots: &mut [Option<input::SmoothedAnalog<'static>>; MAX_ANALOG_INPUTS],
    ldr: &mut Option<input::SmoothedAnalog<'static>>,
) {
    let bg: [u8; MAX_DIGITAL_INPUTS] = collect_field(cfg.active_buttons(), 0, |b| b.pin);
    buttons.configure(&bg[..cfg.num_buttons as usize]);

    let tg: [u8; MAX_DIGITAL_INPUTS] = collect_field(cfg.active_touch_pads(), 0, |t| t.pin);
    let tt: [u8; MAX_DIGITAL_INPUTS] =
        collect_field(cfg.active_touch_pads(), 25, |t| t.threshold_pct);
    touch.configure(
        &tg[..cfg.num_touch_pads as usize],
        &tt[..cfg.num_touch_pads as usize],
    );

    for pot in pots.iter_mut() {
        *pot = None;
    }
    for (i, pot_def) in cfg.active_pots().iter().enumerate() {
        if let Some(ch) = input::adc_channel_from_gpio(pot_def.pin) {
            pots[i] = Some(input::SmoothedAnalog::new(ch, 0.2));
        }
    }

    *ldr = None;
    if cfg.ldr_enabled {
        if let Some(ch) = input::adc_channel_from_gpio(cfg.ldr.pin) {
            *ldr = Some(input::SmoothedAnalog::new(ch, 0.15));
        }
    }
}

/// Send a MIDI packet with a timeout to avoid blocking input polling
/// when no MIDI host is actively reading from the device.
async fn send_midi(midi: &mut MidiClass<'static, Driver<'static, USB>>, pkt: &[u8; 4]) {
    const MIDI_TIMEOUT: Duration = Duration::from_millis(5);
    let _ = select(midi.write_packet(pkt), Timer::after(MIDI_TIMEOUT)).await;
}

/// Handle press/release events for note-producing inputs (buttons or
/// touch pads).  `events` is an iterator of `(index, pressed)` pairs.
#[allow(clippy::too_many_arguments)]
async fn handle_note_events(
    events: impl Iterator<Item = (usize, bool)>,
    defs: &[impl NoteInput],
    held: &mut [Option<HeldNote>; MAX_DIGITAL_INPUTS],
    channel: u8,
    expr_inputs: &ExprInputs,
    midi: &mut MidiClass<'static, Driver<'static, USB>>,
    input_state: &InputState,
    set_state: fn(&InputState, u8, bool),
) {
    for (idx, pressed) in events {
        if pressed {
            let def = &defs[idx];
            let ne = def.note_expr();
            let ve = def.velocity_expr();
            let note = expr::eval(&ne.code, ne.len, expr_inputs, def.note());
            let vel = expr::eval(&ve.code, ve.len, expr_inputs, def.velocity());
            held[idx] = Some(HeldNote::new(note));
            send_midi(midi, &note_on(channel, note, vel)).await;
        } else {
            let note = held[idx].map_or(defs[idx].note(), |h| h.current);
            held[idx] = None;
            send_midi(midi, &note_off(channel, note)).await;
        }
        #[allow(clippy::cast_possible_truncation)]
        set_state(input_state, idx as u8, pressed);
    }
}

/// Re-evaluate `scale()` expressions for held notes and retrigger when
/// the quantised note changes after the debounce period.
async fn retrigger_held_notes(
    defs: &[impl NoteInput],
    held: &mut [Option<HeldNote>; MAX_DIGITAL_INPUTS],
    count: usize,
    channel: u8,
    expr_inputs: &ExprInputs,
    midi: &mut MidiClass<'static, Driver<'static, USB>>,
) {
    let now = Instant::now();
    for (idx, slot) in held.iter_mut().take(count).enumerate() {
        let h = match slot {
            Some(h) => h,
            None => continue,
        };
        let def = &defs[idx];
        let ne = def.note_expr();
        if !expr::has_scale(&ne.code, ne.len) {
            continue;
        }
        let note = expr::eval(&ne.code, ne.len, expr_inputs, def.note());
        let old_note = h.current;
        if h.update(note, now) {
            let ve = def.velocity_expr();
            let vel = expr::eval(&ve.code, ve.len, expr_inputs, def.velocity());
            send_midi(midi, &note_off(channel, old_note)).await;
            send_midi(midi, &note_on(channel, note, vel)).await;
        }
    }
}

/// Release all currently held notes, sending note-off for each, and
/// clear the corresponding input state.
async fn release_held_notes(
    held: &mut [Option<HeldNote>; MAX_DIGITAL_INPUTS],
    channel: u8,
    midi: &mut MidiClass<'static, Driver<'static, USB>>,
    input_state: &InputState,
    set_state: fn(&InputState, u8, bool),
) {
    for (idx, slot) in held.iter_mut().enumerate() {
        if let Some(h) = slot.take() {
            send_midi(midi, &note_off(channel, h.current)).await;
            #[allow(clippy::cast_possible_truncation)]
            set_state(input_state, idx as u8, false);
        }
    }
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

    let mut buttons = input::Buttons::new();
    let mut touch = input::TouchPads::new();
    let mut pots: [Option<input::SmoothedAnalog<'static>>; MAX_ANALOG_INPUTS] =
        [const { None }; MAX_ANALOG_INPUTS];
    let mut ldr: Option<input::SmoothedAnalog<'static>> = None;

    let initial_cfg = *cfg.borrow();
    let mut pin_snapshot = PinSnapshot::from_config(&initial_cfg);

    // Safety: we own all the peripheral singletons via `p` and configure
    // each GPIO only once based on validated config (no duplicates).
    unsafe {
        configure_inputs(&initial_cfg, &mut buttons, &mut touch, &mut pots, &mut ldr);
    }

    let accel_dead_zone = initial_cfg.accel.dead_zone_tenths;
    let accel_smoothing = initial_cfg.accel.smoothing_pct;
    let accel_chip = initial_cfg.accel.chip;
    let mut accel =
        input::Accelerometer::new(i2c1, accel_dead_zone, accel_smoothing, accel_chip).await;

    let mut last_led_toggle = Instant::now();

    let mut active_button_note: [Option<HeldNote>; MAX_DIGITAL_INPUTS] = [None; MAX_DIGITAL_INPUTS];
    let mut active_touch_note: [Option<HeldNote>; MAX_DIGITAL_INPUTS] = [None; MAX_DIGITAL_INPUTS];

    loop {
        Timer::after(Duration::from_millis(1)).await;

        let cur = *cfg.borrow();

        let new_snapshot = PinSnapshot::from_config(&cur);
        if new_snapshot != pin_snapshot {
            // Release all held notes before reconfiguring
            release_held_notes(
                &mut active_button_note,
                cur.midi_channel,
                midi_class,
                input_state,
                InputState::set_button,
            )
            .await;
            release_held_notes(
                &mut active_touch_note,
                cur.midi_channel,
                midi_class,
                input_state,
                InputState::set_touch,
            )
            .await;

            // Safety: we're reconfiguring pins based on a validated config.
            // The old pin drivers are dropped inside configure(), releasing
            // the GPIO before the new ones are created.
            unsafe {
                configure_inputs(&cur, &mut buttons, &mut touch, &mut pots, &mut ldr);
            }

            pin_snapshot = new_snapshot;
            defmt::info!(
                "pins reconfigured: {}b {}t {}p",
                cur.num_buttons,
                cur.num_touch_pads,
                cur.num_pots
            );
        }

        accel.update_params(cur.accel.dead_zone_tenths, cur.accel.smoothing_pct);

        let thrs: [u8; MAX_DIGITAL_INPUTS] =
            collect_field(cur.active_touch_pads(), 25, |t| t.threshold_pct);
        touch.update_thresholds(&thrs[..cur.num_touch_pads as usize]);

        // Poll analog/sensor inputs before building expression snapshot
        // so note/velocity expressions see the freshest sensor readings.
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

        if cur.ldr_enabled {
            if let Some(ldr_input) = &mut ldr {
                if let Some(v) = ldr_input.poll(adc_inst, 2).await {
                    let pkt = control_change(cur.midi_channel, cur.ldr.cc, v);
                    send_midi(midi_class, &pkt).await;
                }
                input_state.set_ldr(ldr_input.current_cc());
            }
        }

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
                // Brief delay so MIDI receivers register the note before the
                // immediate note-off.
                Timer::after(Duration::from_millis(10)).await;
                send_midi(midi_class, &note_off(cur.midi_channel, cur.accel.tap_note)).await;
                input_state.set_accel_tap();
            }
            input_state.set_accel_x(accel.current_x_cc());
            input_state.set_accel_y(accel.current_y_cc());
        }

        let expr_inputs = ExprInputs {
            pots: input_state.pots_snapshot(),
            ldr: input_state.ldr_value(),
            accel_x: input_state.accel_x_value(),
            accel_y: input_state.accel_y_value(),
        };

        let num_buttons = cur.num_buttons as usize;
        let button_events = buttons
            .poll()
            .into_iter()
            .take(num_buttons)
            .filter_map(|e| e.map(|e| (e.index as usize, e.pressed)));
        handle_note_events(
            button_events,
            cur.active_buttons(),
            &mut active_button_note,
            cur.midi_channel,
            &expr_inputs,
            midi_class,
            input_state,
            InputState::set_button,
        )
        .await;

        retrigger_held_notes(
            cur.active_buttons(),
            &mut active_button_note,
            num_buttons,
            cur.midi_channel,
            &expr_inputs,
            midi_class,
        )
        .await;

        let num_touch = cur.num_touch_pads as usize;
        let touch_events = touch
            .poll()
            .await
            .into_iter()
            .take(num_touch)
            .filter_map(|e| e.map(|e| (e.index as usize, e.pressed)));

        // Update touch telemetry for the monitor snapshot.
        let telemetry = touch.telemetry();
        for (i, t) in telemetry.iter().take(num_touch).enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            input_state.set_touch_telemetry(i as u8, t.filtered, t.baseline, t.threshold);
        }

        handle_note_events(
            touch_events,
            cur.active_touch_pads(),
            &mut active_touch_note,
            cur.midi_channel,
            &expr_inputs,
            midi_class,
            input_state,
            InputState::set_touch,
        )
        .await;

        retrigger_held_notes(
            cur.active_touch_pads(),
            &mut active_touch_note,
            num_touch,
            cur.midi_channel,
            &expr_inputs,
            midi_class,
        )
        .await;

        if Instant::now().duration_since(last_led_toggle).as_millis() >= 1000 {
            last_led_toggle = Instant::now();
            led.toggle();
        }
    }
}
