//! MIDI controller firmware for RP2040 and RP2350.
//!
//! USB composite device: MIDI + CDC-ACM serial.
//! MIDI: NoteOn/NoteOff for buttons/touch, CC for pots/LDR/accelerometer.
//! Serial: line-based config protocol (GET/PUT/SAVE/RESET/REBOOT) + always-on monitoring.

#![no_std]
#![no_main]

mod config;
mod input;
mod input_state;
mod midi;
mod serial;

use embassy_executor::Spawner;
use embassy_futures::join::join3;
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

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    ADC_IRQ_FIFO => adc::InterruptHandler;
    I2C0_IRQ => i2c::InterruptHandler<I2C0>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // ---- Load config from flash ----
    let mut flash = flash::Flash::<_, flash::Blocking, { config::FLASH_SIZE }>::new_blocking(p.FLASH);
    let mut cfg = serial::load_config(&mut flash).unwrap_or_else(|| {
        defmt::info!("no saved config, using defaults");
        Config::default()
    });
    defmt::info!("config loaded: ch={} buttons={} touch={} pots={}",
        cfg.midi_channel, cfg.num_buttons, cfg.num_touch_pads, cfg.num_pots);

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

    // CDC-ACM serial
    static CDC_STATE: StaticCell<State> = StaticCell::new();
    let mut serial_class = CdcAcmClass::new(&mut builder, CDC_STATE.init(State::new()), 64);

    // MIDI class
    let mut midi_class = MidiClass::new(&mut builder, 1, 1, 64);

    let mut usb = builder.build();

    // ---- Status LED (GP25 on Pico / Pico 2) ----
    let mut led = Output::new(p.PIN_25, Level::Low);

    // ---- ADC for pots/LDR ----
    let mut adc_inst = adc::Adc::new(p.ADC, Irqs, adc::Config::default());

    // ---- Shared input state for live monitoring ----
    static INPUT_STATE: InputState = InputState::new();

    // ---- Run three concurrent tasks ----
    let usb_fut = usb.run();

    // Clone config for the MIDI task (read-only snapshot).
    // Config changes via serial require a reboot to take effect since
    // hardware pin setup happens at init time.
    let midi_cfg = cfg;

    let midi_fut = async {
        Timer::after(Duration::from_millis(100)).await;

        // --- Buttons: GP2-GP5 ---
        let mut buttons = input::Buttons::new([
            Input::new(p.PIN_2, Pull::Up),
            Input::new(p.PIN_3, Pull::Up),
            Input::new(p.PIN_4, Pull::Up),
            Input::new(p.PIN_5, Pull::Up),
        ]);

        // --- Touch pads: GP6-GP10 ---
        let mut touch_pins: [Flex<'static>; 5] = [
            Flex::new(p.PIN_6),
            Flex::new(p.PIN_7),
            Flex::new(p.PIN_8),
            Flex::new(p.PIN_9),
            Flex::new(p.PIN_10),
        ];
        let mut touch = input::TouchPads::new(&mut touch_pins);

        // --- Pots: GP26 (ADC0), GP27 (ADC1) ---
        let mut pot0 = input::SmoothedAnalog::new(
            adc::Channel::new_pin(p.PIN_26, Pull::None), 0.2,
        );
        let mut pot1 = input::SmoothedAnalog::new(
            adc::Channel::new_pin(p.PIN_27, Pull::None), 0.2,
        );

        // --- LDR: GP28 (ADC2) ---
        let mut ldr = input::SmoothedAnalog::new(
            adc::Channel::new_pin(p.PIN_28, Pull::None), 0.15,
        );

        // --- Accelerometer: I2C0, SCL=GP1, SDA=GP0 ---
        let i2c0 = i2c::I2c::new_async(p.I2C0, p.PIN_1, p.PIN_0, Irqs, i2c::Config::default());
        let mut accel = input::Accelerometer::new(
            i2c0, midi_cfg.accel.dead_zone_tenths, midi_cfg.accel.smoothing_pct,
        ).await;

        let mut last_led_toggle = Instant::now();
        let mut led_on = false;

        // Timeout for MIDI writes: prevents blocking input polling when
        // no MIDI host is actively reading from the device.
        let midi_timeout = Duration::from_millis(5);

        loop {
            // ~1ms poll interval
            Timer::after(Duration::from_millis(1)).await;

            // Poll buttons
            let nb = (midi_cfg.num_buttons as usize).min(4);
            for evt in buttons.poll().into_iter().flatten() {
                if (evt.index as usize) < nb {
                    let def = &midi_cfg.buttons[evt.index as usize];
                    let pkt = if evt.pressed {
                        midi::note_on(midi_cfg.midi_channel, def.note, def.velocity)
                    } else {
                        midi::note_off(midi_cfg.midi_channel, def.note)
                    };
                    let _ = embassy_futures::select::select(
                        midi_class.write_packet(&pkt),
                        Timer::after(midi_timeout),
                    ).await;
                }
                INPUT_STATE.set_button(evt.index, evt.pressed);
            }

            // Poll touch pads
            let nt = (midi_cfg.num_touch_pads as usize).min(5);
            for evt in touch.poll(&mut touch_pins).into_iter().flatten() {
                if (evt.index as usize) < nt {
                    let def = &midi_cfg.touch_pads[evt.index as usize];
                    let pkt = if evt.pressed {
                        midi::note_on(midi_cfg.midi_channel, def.note, def.velocity)
                    } else {
                        midi::note_off(midi_cfg.midi_channel, def.note)
                    };
                    let _ = embassy_futures::select::select(
                        midi_class.write_packet(&pkt),
                        Timer::after(midi_timeout),
                    ).await;
                }
                INPUT_STATE.set_touch(evt.index, evt.pressed);
            }

            // Poll pots
            if midi_cfg.num_pots >= 1 {
                if let Some(v) = pot0.poll(&mut adc_inst, 2).await {
                    let pkt = midi::control_change(midi_cfg.midi_channel, midi_cfg.pots[0].cc, v);
                    let _ = embassy_futures::select::select(
                        midi_class.write_packet(&pkt),
                        Timer::after(midi_timeout),
                    ).await;
                }
                INPUT_STATE.set_pot(0, pot0.current_cc());
            }
            if midi_cfg.num_pots >= 2 {
                if let Some(v) = pot1.poll(&mut adc_inst, 2).await {
                    let pkt = midi::control_change(midi_cfg.midi_channel, midi_cfg.pots[1].cc, v);
                    let _ = embassy_futures::select::select(
                        midi_class.write_packet(&pkt),
                        Timer::after(midi_timeout),
                    ).await;
                }
                INPUT_STATE.set_pot(1, pot1.current_cc());
            }

            // Poll LDR
            if midi_cfg.ldr_enabled {
                if let Some(v) = ldr.poll(&mut adc_inst, 2).await {
                    let pkt = midi::control_change(midi_cfg.midi_channel, midi_cfg.ldr.cc, v);
                    let _ = embassy_futures::select::select(
                        midi_class.write_packet(&pkt),
                        Timer::after(midi_timeout),
                    ).await;
                }
                INPUT_STATE.set_ldr(ldr.current_cc());
            }

            // Poll accelerometer
            if midi_cfg.accel.enabled && accel.available {
                let r = accel.poll().await;
                if let Some(x) = r.x_cc {
                    let _ = embassy_futures::select::select(
                        midi_class.write_packet(&midi::control_change(midi_cfg.midi_channel, midi_cfg.accel.x_cc, x)),
                        Timer::after(midi_timeout),
                    ).await;
                }
                if let Some(y) = r.y_cc {
                    let _ = embassy_futures::select::select(
                        midi_class.write_packet(&midi::control_change(midi_cfg.midi_channel, midi_cfg.accel.y_cc, y)),
                        Timer::after(midi_timeout),
                    ).await;
                }
                if r.tapped {
                    let _ = embassy_futures::select::select(
                        midi_class.write_packet(&midi::note_on(midi_cfg.midi_channel, midi_cfg.accel.tap_note, midi_cfg.accel.tap_velocity)),
                        Timer::after(midi_timeout),
                    ).await;
                    let _ = embassy_futures::select::select(
                        midi_class.write_packet(&midi::note_off(midi_cfg.midi_channel, midi_cfg.accel.tap_note)),
                        Timer::after(midi_timeout),
                    ).await;
                    INPUT_STATE.set_accel_tap();
                }
                INPUT_STATE.set_accel_x(accel.current_x_cc());
                INPUT_STATE.set_accel_y(accel.current_y_cc());
            }

            // Blink status LED every 1s
            if Instant::now().duration_since(last_led_toggle).as_millis() >= 1000 {
                last_led_toggle = Instant::now();
                led_on = !led_on;
                if led_on { led.set_high(); } else { led.set_low(); }
            }
        }
    };

    let serial_fut = async {
        loop {
            serial_class.wait_connection().await;
            defmt::info!("serial connected");

            let mut line_buf = [0u8; 256];
            let mut line_pos = 0usize;
            let mut last_monitor_send = Instant::now();

            loop {
                // Always interleave command processing with monitor snapshots.
                // Use select to either read a packet or timeout for next monitor send.
                let mut buf = [0u8; 64];

                let read_or_tick = embassy_futures::select::select(
                    serial_class.read_packet(&mut buf),
                    Timer::after(Duration::from_millis(10)),
                ).await;

                match read_or_tick {
                    embassy_futures::select::Either::First(result) => {
                        match result {
                            Ok(n) => {
                                for &b in &buf[..n] {
                                    if b == b'\n' || b == b'\r' {
                                        if line_pos > 0 {
                                            let mut resp = [0u8; 1024];
                                            let (resp_len, action) = serial::handle_command(
                                                &line_buf[..line_pos], &mut cfg, &mut resp,
                                            );
                                            // For SAVE, perform flash write before sending response
                                            if action == serial::Action::Save {
                                                if serial::save_config(&mut flash, &cfg) {
                                                    let ok = b"OK saved\n";
                                                    let _ = serial_class.write_packet(ok).await;
                                                } else {
                                                    let err = b"ERR save failed\n";
                                                    let _ = serial_class.write_packet(err).await;
                                                }
                                            } else {
                                                // Send response in 64-byte chunks
                                                let mut sent = 0;
                                                while sent < resp_len {
                                                    let end = (sent + 64).min(resp_len);
                                                    if serial_class.write_packet(&resp[sent..end]).await.is_err() {
                                                        break;
                                                    }
                                                    sent = end;
                                                }
                                            }
                                            if action == serial::Action::Reboot {
                                                // Allow USB to flush before reset
                                                Timer::after(Duration::from_millis(50)).await;
                                                cortex_m::peripheral::SCB::sys_reset();
                                            }
                                            line_pos = 0;
                                        }
                                    } else if line_pos < line_buf.len() {
                                        line_buf[line_pos] = b;
                                        line_pos += 1;
                                    }
                                }
                            }
                            Err(EndpointError::Disabled) => break,
                            Err(EndpointError::BufferOverflow) => {
                                defmt::warn!("serial overflow");
                            }
                        }
                    }
                    embassy_futures::select::Either::Second(()) => {}
                }

                // Always send monitor snapshot at ~50ms intervals
                if Instant::now().duration_since(last_monitor_send).as_millis() >= 50 {
                    last_monitor_send = Instant::now();
                    let mut resp = [0u8; 256];
                    let n = INPUT_STATE.format_snapshot(&mut resp);
                    if n > 0 {
                        let mut sent = 0;
                        while sent < n {
                            let end = (sent + 64).min(n);
                            if serial_class.write_packet(&resp[sent..end]).await.is_err() {
                                break;
                            }
                            sent = end;
                        }
                    }
                }
            }
            defmt::info!("serial disconnected");
        }
    };

    join3(usb_fut, midi_fut, serial_fut).await;
}
