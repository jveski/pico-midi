#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
// Embassy uses a single-threaded executor; futures do not need to be Send.
#![allow(clippy::future_not_send)]

mod config;
mod expr;
#[cfg(target_os = "none")]
mod input;
#[cfg(target_os = "none")]
mod input_state;
#[cfg(target_os = "none")]
mod polling;
mod serial;

#[cfg(target_os = "none")]
use core::cell::RefCell;

#[cfg(target_os = "none")]
use embassy_executor::Spawner;
#[cfg(target_os = "none")]
use embassy_futures::join::join3;
#[cfg(target_os = "none")]
use embassy_rp::adc;
#[cfg(target_os = "none")]
use embassy_rp::bind_interrupts;
#[cfg(target_os = "none")]
use embassy_rp::flash;
#[cfg(target_os = "none")]
use embassy_rp::gpio::{Level, Output};
#[cfg(target_os = "none")]
use embassy_rp::i2c;
#[cfg(target_os = "none")]
use embassy_rp::peripherals::{I2C1, USB};
#[cfg(target_os = "none")]
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
#[cfg(target_os = "none")]
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
#[cfg(target_os = "none")]
use embassy_usb::class::midi::MidiClass;
#[cfg(target_os = "none")]
use embassy_usb::{Builder, Config as UsbConfig};
#[cfg(target_os = "none")]
use static_cell::StaticCell;
#[cfg(target_os = "none")]
use {defmt_rtt as _, panic_probe as _};

#[cfg(target_os = "none")]
use config::Config;
#[cfg(target_os = "none")]
use input_state::InputState;

#[cfg(target_os = "none")]
bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    ADC_IRQ_FIFO => adc::InterruptHandler;
    I2C1_IRQ => i2c::InterruptHandler<I2C1>;
});

#[cfg(target_os = "none")]
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(embassy_rp::config::Config::default());

    let mut flash =
        flash::Flash::<_, flash::Blocking, { config::FLASH_SIZE }>::new_blocking(p.FLASH);
    let cfg = config::load_config(&mut flash).unwrap_or_else(|| {
        defmt::info!("no saved config, using defaults");
        Config::default()
    });
    defmt::info!("config loaded: ch={}", cfg.midi_channel);

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

    let cfg = RefCell::new(cfg);

    let i2c1 = i2c::I2c::new_async(p.I2C1, p.PIN_3, p.PIN_2, Irqs, i2c::Config::default());
    let midi_fut = polling::run(
        &mut midi_class,
        &mut led,
        &mut adc_inst,
        i2c1,
        &cfg,
        &INPUT_STATE,
    );

    let serial_fut = serial::serial_task(&mut serial_class, &mut flash, &cfg, &INPUT_STATE);

    join3(usb_fut, midi_fut, serial_fut).await;
}
