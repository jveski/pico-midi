#![no_std]
#![no_main]

mod config;
mod input;
mod input_state;
mod serial;

use embassy_executor::Spawner;
use embassy_futures::join::join3;
use embassy_futures::select::{select, Either};
use embassy_rp::adc;
use embassy_rp::bind_interrupts;
use embassy_rp::flash;
use embassy_rp::gpio::{Flex, Input, Level, Output, Pull};
use embassy_rp::i2c;
use embassy_rp::peripherals::{I2C0, USB};
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use embassy_time::{Duration, Instant, Timer};
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::class::midi::MidiClass;
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, Config as UsbConfig};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use config::Config;
use input_state::InputState;

const CIN_NOTE_OFF: u8 = 0x08;
const CIN_NOTE_ON: u8 = 0x09;
const CIN_CC: u8 = 0x0B;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    ADC_IRQ_FIFO => adc::InterruptHandler;
    I2C0_IRQ => i2c::InterruptHandler<I2C0>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // ---- Load config from flash ----
    let mut flash =
        flash::Flash::<_, flash::Blocking, { config::FLASH_SIZE }>::new_blocking(p.FLASH);
    let mut cfg = serial::load_config(&mut flash).unwrap_or_else(|| {
        defmt::info!("no saved config, using defaults");
        Config::default()
    });
    defmt::info!(
        "config loaded: ch={} buttons={} touch={} pots={}",
        cfg.midi_channel,
        cfg.num_buttons,
        cfg.num_touch_pads,
        cfg.num_pots
    );

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

    // Clone config for the MIDI task (read-only snapshot).
    // Config changes via serial require a reboot to take effect since
    // hardware pin setup happens at init time.
    let midi_cfg = cfg;

    let midi_fut = async {
        Timer::after(Duration::from_millis(100)).await;

        // --- Buttons: GP2-GP5, GP11-GP14 ---
        let mut buttons = input::Buttons::new([
            Input::new(p.PIN_2, Pull::Up),
            Input::new(p.PIN_3, Pull::Up),
            Input::new(p.PIN_4, Pull::Up),
            Input::new(p.PIN_5, Pull::Up),
            Input::new(p.PIN_11, Pull::Up),
            Input::new(p.PIN_12, Pull::Up),
            Input::new(p.PIN_13, Pull::Up),
            Input::new(p.PIN_14, Pull::Up),
        ]);

        // --- Touch pads: GP6-GP10, GP15-GP17 ---
        let mut touch_pins: [Flex<'static>; 8] = [
            Flex::new(p.PIN_6),
            Flex::new(p.PIN_7),
            Flex::new(p.PIN_8),
            Flex::new(p.PIN_9),
            Flex::new(p.PIN_10),
            Flex::new(p.PIN_15),
            Flex::new(p.PIN_16),
            Flex::new(p.PIN_17),
        ];
        let mut touch = input::TouchPads::new(&mut touch_pins);

        // --- Pots: GP26 (ADC0), GP27 (ADC1) ---
        let mut pots = [
            input::SmoothedAnalog::new(adc::Channel::new_pin(p.PIN_26, Pull::None), 0.2),
            input::SmoothedAnalog::new(adc::Channel::new_pin(p.PIN_27, Pull::None), 0.2),
        ];

        // --- LDR: GP28 (ADC2) ---
        let mut ldr = input::SmoothedAnalog::new(adc::Channel::new_pin(p.PIN_28, Pull::None), 0.15);

        // --- Accelerometer: I2C0, SCL=GP1, SDA=GP0 ---
        let i2c0 = i2c::I2c::new_async(p.I2C0, p.PIN_1, p.PIN_0, Irqs, i2c::Config::default());
        let mut accel = input::Accelerometer::new(
            i2c0,
            midi_cfg.accel.dead_zone_tenths,
            midi_cfg.accel.smoothing_pct,
        )
        .await;

        let mut last_led_toggle = Instant::now();
        let mut led_on = false;

        loop {
            // ~1ms poll interval
            Timer::after(Duration::from_millis(1)).await;

            // Poll buttons
            let nb = (midi_cfg.num_buttons as usize).min(8);
            for evt in buttons.poll().into_iter().flatten() {
                if (evt.index as usize) < nb {
                    let def = &midi_cfg.buttons[evt.index as usize];
                    let pkt = if evt.pressed {
                        note_on(midi_cfg.midi_channel, def.note, def.velocity)
                    } else {
                        note_off(midi_cfg.midi_channel, def.note)
                    };
                    send_midi(&mut midi_class, &pkt).await;
                }
                INPUT_STATE.set_button(evt.index, evt.pressed);
            }

            // Poll touch pads
            let nt = (midi_cfg.num_touch_pads as usize).min(8);
            for evt in touch.poll(&mut touch_pins).await.into_iter().flatten() {
                if (evt.index as usize) < nt {
                    let def = &midi_cfg.touch_pads[evt.index as usize];
                    let pkt = if evt.pressed {
                        note_on(midi_cfg.midi_channel, def.note, def.velocity)
                    } else {
                        note_off(midi_cfg.midi_channel, def.note)
                    };
                    send_midi(&mut midi_class, &pkt).await;
                }
                INPUT_STATE.set_touch(evt.index, evt.pressed);
            }

            // Poll pots
            for i in 0..(midi_cfg.num_pots as usize).min(pots.len()) {
                if let Some(v) = pots[i].poll(&mut adc_inst, 2).await {
                    let pkt = control_change(midi_cfg.midi_channel, midi_cfg.pots[i].cc, v);
                    send_midi(&mut midi_class, &pkt).await;
                }
                INPUT_STATE.set_pot(i as u8, pots[i].current_cc());
            }

            // Poll LDR
            if midi_cfg.ldr_enabled {
                if let Some(v) = ldr.poll(&mut adc_inst, 2).await {
                    let pkt = control_change(midi_cfg.midi_channel, midi_cfg.ldr.cc, v);
                    send_midi(&mut midi_class, &pkt).await;
                }
                INPUT_STATE.set_ldr(ldr.current_cc());
            }

            // Poll accelerometer
            if midi_cfg.accel.enabled && accel.available {
                let r = accel.poll().await;
                if let Some(x) = r.x_cc {
                    send_midi(
                        &mut midi_class,
                        &control_change(midi_cfg.midi_channel, midi_cfg.accel.x_cc, x),
                    )
                    .await;
                }
                if let Some(y) = r.y_cc {
                    send_midi(
                        &mut midi_class,
                        &control_change(midi_cfg.midi_channel, midi_cfg.accel.y_cc, y),
                    )
                    .await;
                }
                if r.tapped {
                    send_midi(
                        &mut midi_class,
                        &note_on(
                            midi_cfg.midi_channel,
                            midi_cfg.accel.tap_note,
                            midi_cfg.accel.tap_velocity,
                        ),
                    )
                    .await;
                    send_midi(
                        &mut midi_class,
                        &note_off(midi_cfg.midi_channel, midi_cfg.accel.tap_note),
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

            let mut frame_buf = [0u8; 256];
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
                                            let mut resp = [0u8; 1024];
                                            let (resp_len, action) = serial::handle_frame(
                                                &mut frame_buf[..frame_pos],
                                                &mut cfg,
                                                &mut resp,
                                            );
                                            if action == serial::Action::Save {
                                                if serial::save_config(&mut flash, &cfg) {
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
                    let snapshot = INPUT_STATE.snapshot();
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
fn note_on(channel: u8, note: u8, velocity: u8) -> [u8; 4] {
    [
        CIN_NOTE_ON,
        0x90 | (channel & 0x0F),
        note & 0x7F,
        velocity & 0x7F,
    ]
}

#[inline]
fn note_off(channel: u8, note: u8) -> [u8; 4] {
    [CIN_NOTE_OFF, 0x80 | (channel & 0x0F), note & 0x7F, 0]
}

#[inline]
fn control_change(channel: u8, cc: u8, value: u8) -> [u8; 4] {
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
