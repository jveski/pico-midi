#![no_std]
#![no_main]
// Embassy uses a single-threaded executor; futures do not need to be Send.
#![allow(clippy::future_not_send)]

mod config;
mod expr;
mod input;
mod input_state;
mod serial;

use core::cell::RefCell;

use embassy_executor::Spawner;
use embassy_futures::join::join3;
use embassy_futures::select::{select, Either};
use embassy_rp::adc;
use embassy_rp::bind_interrupts;
use embassy_rp::flash;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c;
use embassy_rp::peripherals::{I2C1, USB};
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use embassy_time::{Duration, Instant, Timer};
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::class::midi::MidiClass;
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, Config as UsbConfig};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use config::{Config, MAX_ANALOG_INPUTS, MAX_DIGITAL_INPUTS};
use expr::ExprInputs;
use input_state::InputState;

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

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    ADC_IRQ_FIFO => adc::InterruptHandler;
    I2C1_IRQ => i2c::InterruptHandler<I2C1>;
});

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

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let _ = spawner; // unused but required by embassy macro signature
    let p = embassy_rp::init(embassy_rp::config::Config::default());

    // ---- Load config from flash ----
    let mut flash =
        flash::Flash::<_, flash::Blocking, { config::FLASH_SIZE }>::new_blocking(p.FLASH);
    let cfg = serial::load_config(&mut flash).unwrap_or_else(|| {
        defmt::info!("no saved config, using defaults");
        Config::default()
    });
    defmt::info!("config loaded: ch={}", cfg.midi_channel);

    // ---- USB composite device setup ----
    let driver = Driver::new(p.USB, Irqs);

    let mut usb_config = UsbConfig::new(0x1209, 0x0001);
    usb_config.manufacturer = Some("MIDICtrl");
    usb_config.product = Some("MIDI Controller");
    usb_config.serial_number = Some("00000001");
    usb_config.max_power = 100;
    usb_config.max_packet_size_0 = 64;

    static CONFIG_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static MSOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();

    let mut builder = Builder::new(
        driver,
        usb_config,
        CONFIG_DESC.init([0; 256]),
        BOS_DESC.init([0; 256]),
        MSOS_DESC.init([0; 256]),
        CONTROL_BUF.init([0; 64]),
    );

    static CDC_STATE: StaticCell<State> = StaticCell::new();
    let mut serial_class = CdcAcmClass::new(&mut builder, CDC_STATE.init(State::new()), 64);
    let mut midi_class = MidiClass::new(&mut builder, 1, 1, 64);
    let mut usb = builder.build();
    let mut led = Output::new(p.PIN_25, Level::Low);
    let mut adc_inst = adc::Adc::new(p.ADC, Irqs, adc::Config::default());
    static INPUT_STATE: InputState = InputState::new();
    let usb_fut = usb.run();

    // Shared config accessible by both the MIDI and serial futures.
    // Embassy uses a single-threaded executor so RefCell is safe.
    let cfg = RefCell::new(cfg);

    let midi_fut = async {
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
        let i2c1 = i2c::I2c::new_async(p.I2C1, p.PIN_3, p.PIN_2, Irqs, i2c::Config::default());
        let accel_dead_zone = initial_cfg.accel.dead_zone_tenths;
        let accel_smoothing = initial_cfg.accel.smoothing_pct;
        let mut accel = input::Accelerometer::new(i2c1, accel_dead_zone, accel_smoothing).await;

        let mut last_led_toggle = Instant::now();
        let mut led_on = false;

        // Track the note currently sounding for each held button/pad so
        // that (a) note-off always releases the correct note, and (b) when
        // a scale() expression result changes while held the old note is
        // released and the new one is sent after a debounce period.
        let mut active_button_note: [Option<HeldNote>; MAX_DIGITAL_INPUTS] =
            [None; MAX_DIGITAL_INPUTS];
        let mut active_touch_note: [Option<HeldNote>; MAX_DIGITAL_INPUTS] =
            [None; MAX_DIGITAL_INPUTS];

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
                        send_midi(&mut midi_class, &note_off(cur.midi_channel, held.current)).await;
                        INPUT_STATE.set_button(idx as u8, false);
                    }
                }
                for (idx, slot) in active_touch_note.iter_mut().enumerate() {
                    if let Some(held) = slot.take() {
                        send_midi(&mut midi_class, &note_off(cur.midi_channel, held.current)).await;
                        INPUT_STATE.set_touch(idx as u8, false);
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
                pots: INPUT_STATE.pots_snapshot(),
                ldr: INPUT_STATE.ldr_value(),
                accel_x: INPUT_STATE.accel_x_value(),
                accel_y: INPUT_STATE.accel_y_value(),
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
                    send_midi(&mut midi_class, &note_on(cur.midi_channel, note, vel)).await;
                } else {
                    let note = active_button_note[idx].map_or(cur.buttons[idx].note, |h| h.current);
                    active_button_note[idx] = None;
                    send_midi(&mut midi_class, &note_off(cur.midi_channel, note)).await;
                };
                INPUT_STATE.set_button(evt.index, evt.pressed);
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
                        if now.duration_since(held.pending_since).as_millis()
                            >= RETRIGGER_DEBOUNCE_MS
                        {
                            let vel = expr::eval(
                                &def.velocity_expr.code,
                                def.velocity_expr.len,
                                &expr_inputs,
                                def.velocity,
                            );
                            send_midi(&mut midi_class, &note_off(cur.midi_channel, held.current))
                                .await;
                            send_midi(&mut midi_class, &note_on(cur.midi_channel, note, vel)).await;
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
                    send_midi(&mut midi_class, &note_on(cur.midi_channel, note, vel)).await;
                } else {
                    let note =
                        active_touch_note[idx].map_or(cur.touch_pads[idx].note, |h| h.current);
                    active_touch_note[idx] = None;
                    send_midi(&mut midi_class, &note_off(cur.midi_channel, note)).await;
                };
                INPUT_STATE.set_touch(evt.index, evt.pressed);
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
                        if now.duration_since(held.pending_since).as_millis()
                            >= RETRIGGER_DEBOUNCE_MS
                        {
                            let vel = expr::eval(
                                &def.velocity_expr.code,
                                def.velocity_expr.len,
                                &expr_inputs,
                                def.velocity,
                            );
                            send_midi(&mut midi_class, &note_off(cur.midi_channel, held.current))
                                .await;
                            send_midi(&mut midi_class, &note_on(cur.midi_channel, note, vel)).await;
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
                    if let Some(v) = pot.poll(&mut adc_inst, 2).await {
                        let pkt = control_change(cur.midi_channel, cur.pots[i].cc, v);
                        send_midi(&mut midi_class, &pkt).await;
                    }
                    #[allow(clippy::cast_possible_truncation)] // index fits in u8
                    INPUT_STATE.set_pot(i as u8, pot.current_cc());
                }
            }

            // Poll LDR
            if cur.ldr_enabled {
                if let Some(ldr_input) = &mut ldr {
                    if let Some(v) = ldr_input.poll(&mut adc_inst, 2).await {
                        let pkt = control_change(cur.midi_channel, cur.ldr.cc, v);
                        send_midi(&mut midi_class, &pkt).await;
                    }
                    INPUT_STATE.set_ldr(ldr_input.current_cc());
                }
            }

            // Poll accelerometer
            if cur.accel.enabled && accel.available {
                let r = accel.poll().await;
                if let Some(x) = r.x_cc {
                    send_midi(
                        &mut midi_class,
                        &control_change(cur.midi_channel, cur.accel.x_cc, x),
                    )
                    .await;
                }
                if let Some(y) = r.y_cc {
                    send_midi(
                        &mut midi_class,
                        &control_change(cur.midi_channel, cur.accel.y_cc, y),
                    )
                    .await;
                }
                if r.tapped {
                    send_midi(
                        &mut midi_class,
                        &note_on(cur.midi_channel, cur.accel.tap_note, cur.accel.tap_velocity),
                    )
                    .await;
                    send_midi(
                        &mut midi_class,
                        &note_off(cur.midi_channel, cur.accel.tap_note),
                    )
                    .await;
                    INPUT_STATE.set_accel_tap();
                }
                INPUT_STATE.set_accel_x(accel.current_x_cc());
                INPUT_STATE.set_accel_y(accel.current_y_cc());
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
    };

    let serial_fut = async {
        loop {
            serial_class.wait_connection().await;
            defmt::info!("serial connected");

            let mut frame_buf = [0u8; 2048];
            let mut frame_pos = 0usize;
            let mut last_monitor_send = Instant::now();

            loop {
                // Interleave command processing with monitor snapshots.
                let mut buf = [0u8; 64];

                let read_or_tick = select(
                    serial_class.read_packet(&mut buf),
                    Timer::after(Duration::from_millis(10)),
                )
                .await;

                match read_or_tick {
                    Either::First(result) => {
                        match result {
                            Ok(n) => {
                                for &b in &buf[..n] {
                                    if b == 0x00 {
                                        // End of COBS frame
                                        if frame_pos > 0 {
                                            let mut resp = [0u8; 2048];
                                            let (resp_len, action) = serial::handle_frame(
                                                &mut frame_buf[..frame_pos],
                                                &mut cfg.borrow_mut(),
                                                &mut resp,
                                            );
                                            if action == serial::Action::Save {
                                                if serial::save_config(&mut flash, &cfg.borrow()) {
                                                    send_serial(
                                                        &mut serial_class,
                                                        &resp[..resp_len],
                                                    )
                                                    .await;
                                                } else {
                                                    let mut err_resp = [0u8; 64];
                                                    let n = serial::encode_error(
                                                        "save failed",
                                                        &mut err_resp,
                                                    );
                                                    if n > 0 {
                                                        send_serial(
                                                            &mut serial_class,
                                                            &err_resp[..n],
                                                        )
                                                        .await;
                                                    }
                                                }
                                            } else if resp_len > 0 {
                                                send_serial(&mut serial_class, &resp[..resp_len])
                                                    .await;
                                            }
                                            frame_pos = 0;
                                        }
                                    } else if frame_pos < frame_buf.len() {
                                        frame_buf[frame_pos] = b;
                                        frame_pos += 1;
                                    }
                                }
                            }
                            Err(EndpointError::Disabled) => break,
                            Err(EndpointError::BufferOverflow) => {
                                defmt::warn!("serial overflow");
                            }
                        }
                    }
                    Either::Second(()) => {}
                }

                // Send monitor snapshot at ~50ms intervals
                if Instant::now().duration_since(last_monitor_send).as_millis() >= 50 {
                    last_monitor_send = Instant::now();
                    let mut resp = [0u8; 256];
                    let snapshot = {
                        let cur = cfg.borrow();
                        INPUT_STATE.snapshot(cur.num_buttons, cur.num_touch_pads, cur.num_pots)
                    };
                    let n = serial::encode_monitor(snapshot, &mut resp);
                    if n > 0 {
                        send_serial(&mut serial_class, &resp[..n]).await;
                    }
                }
            }
            defmt::info!("serial disconnected");
        }
    };

    join3(usb_fut, midi_fut, serial_fut).await;
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

/// Write a byte slice over CDC-ACM serial in 64-byte chunks.
/// Returns `true` if the entire payload was sent successfully.
async fn send_serial(serial: &mut CdcAcmClass<'static, Driver<'static, USB>>, data: &[u8]) -> bool {
    let mut sent = 0;
    while sent < data.len() {
        let end = (sent + 64).min(data.len());
        if serial.write_packet(&data[sent..end]).await.is_err() {
            return false;
        }
        sent = end;
    }
    // Send ZLP if the payload was a non-zero exact multiple of 64 bytes
    if !data.is_empty() && data.len().is_multiple_of(64) && serial.write_packet(&[]).await.is_err()
    {
        return false;
    }
    true
}
