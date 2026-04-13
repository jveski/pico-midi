use embassy_rp::adc::{self, Adc, Channel};
use embassy_rp::gpio::{Drive, Flex, Input, Pull};
use embassy_rp::i2c::{self, I2c};
use embassy_rp::peripherals::I2C1;
use embassy_time::{Instant, Timer};

use crate::config::MAX_DIGITAL_INPUTS;

const DEBOUNCE_MS: u64 = 10;

pub struct ButtonEvent {
    pub index: u8,
    pub pressed: bool,
}

pub struct Buttons {
    pins: [Option<Input<'static>>; MAX_DIGITAL_INPUTS],
    prev: [bool; MAX_DIGITAL_INPUTS],
    stable_since: [Instant; MAX_DIGITAL_INPUTS],
    count: usize,
}

impl Buttons {
    pub fn new() -> Self {
        Self {
            pins: [const { None }; MAX_DIGITAL_INPUTS],
            prev: [false; MAX_DIGITAL_INPUTS],
            stable_since: [Instant::MIN; MAX_DIGITAL_INPUTS],
            count: 0,
        }
    }

    /// # Safety
    /// Caller must ensure the GPIO numbers are valid and not in use elsewhere.
    pub unsafe fn configure(&mut self, gpio_pins: &[u8]) {
        for pin in self.pins.iter_mut() {
            *pin = None;
        }
        self.count = gpio_pins.len().min(MAX_DIGITAL_INPUTS);
        let now = Instant::now();
        for (i, &gpio) in gpio_pins.iter().take(self.count).enumerate() {
            let any = embassy_rp::gpio::AnyPin::steal(gpio);
            self.pins[i] = Some(Input::new(any, Pull::Up));
            self.prev[i] = false;
            self.stable_since[i] = now;
        }
    }

    pub fn poll(&mut self) -> [Option<ButtonEvent>; MAX_DIGITAL_INPUTS] {
        let now = Instant::now();
        let mut events: [Option<ButtonEvent>; MAX_DIGITAL_INPUTS] =
            [const { None }; MAX_DIGITAL_INPUTS];
        for (i, event) in events.iter_mut().enumerate().take(self.count) {
            if let Some(pin) = &self.pins[i] {
                let pressed = pin.is_low();
                if pressed != self.prev[i]
                    && now.duration_since(self.stable_since[i]).as_millis() >= DEBOUNCE_MS
                {
                    self.prev[i] = pressed;
                    self.stable_since[i] = now;
                    *event = Some(ButtonEvent {
                        #[allow(clippy::cast_possible_truncation)]
                        index: i as u8,
                        pressed,
                    });
                }
            }
        }
        events
    }
}

pub struct SmoothedAnalog<'d> {
    channel: Channel<'d>,
    smoothed: f32,
    last_cc: Option<u8>,
    alpha: f32,
}

impl<'d> SmoothedAnalog<'d> {
    pub const fn new(channel: Channel<'d>, alpha: f32) -> Self {
        Self {
            channel,
            smoothed: 0.0,
            last_cc: None,
            alpha,
        }
    }

    pub async fn poll(&mut self, adc: &mut Adc<'static, adc::Async>, threshold: u8) -> Option<u8> {
        let raw = adc.read(&mut self.channel).await.unwrap_or(0);
        self.smoothed = self.alpha * f32::from(raw) + (1.0 - self.alpha) * self.smoothed;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let cc = ((self.smoothed as u32) >> 5).min(127) as u8;

        match self.last_cc {
            None => {
                self.last_cc = Some(cc);
                Some(cc)
            }
            Some(prev) => {
                if cc.abs_diff(prev) >= threshold {
                    self.last_cc = Some(cc);
                    Some(cc)
                } else {
                    None
                }
            }
        }
    }

    pub fn current_cc(&self) -> u8 {
        self.last_cc.unwrap_or(0)
    }
}

pub struct TouchEvent {
    pub index: u8,
    pub pressed: bool,
}

struct TouchPad {
    baseline: u32,
    threshold: u32,
    was_touched: bool,
}

pub struct TouchPads {
    pads: [TouchPad; MAX_DIGITAL_INPUTS],
    pins: [Option<Flex<'static>>; MAX_DIGITAL_INPUTS],
    count: usize,
}

impl TouchPads {
    pub fn new() -> Self {
        Self {
            pads: [const {
                TouchPad {
                    baseline: 0,
                    threshold: 0,
                    was_touched: false,
                }
            }; MAX_DIGITAL_INPUTS],
            pins: [const { None }; MAX_DIGITAL_INPUTS],
            count: 0,
        }
    }

    /// # Safety
    /// Caller must ensure the GPIO numbers are valid and not in use elsewhere.
    pub unsafe fn configure(&mut self, gpio_pins: &[u8], threshold_pcts: &[u8]) {
        for pin in self.pins.iter_mut() {
            *pin = None;
        }
        self.count = gpio_pins.len().min(MAX_DIGITAL_INPUTS);

        for (i, &gpio) in gpio_pins.iter().take(self.count).enumerate() {
            let any = embassy_rp::gpio::AnyPin::steal(gpio);
            let mut flex = Flex::new(any);

            flex.set_schmitt(true);
            flex.set_drive_strength(Drive::_4mA);

            let mut sum: u32 = 0;
            for _ in 0..8 {
                sum += measure_touch_sync(&mut flex);
            }
            let baseline = sum / 8;
            let pct = u32::from(threshold_pcts.get(i).copied().unwrap_or(33));
            let margin = (baseline * pct / 100).max(2);

            self.pads[i] = TouchPad {
                baseline,
                threshold: baseline + margin,
                was_touched: false,
            };
            self.pins[i] = Some(flex);
        }
    }

    pub fn update_thresholds(&mut self, threshold_pcts: &[u8]) {
        for (i, pad) in self.pads.iter_mut().take(self.count).enumerate() {
            let pct = u32::from(threshold_pcts.get(i).copied().unwrap_or(33));
            let margin = (pad.baseline * pct / 100).max(2);
            pad.threshold = pad.baseline + margin;
        }
    }

    pub async fn poll(&mut self) -> [Option<TouchEvent>; MAX_DIGITAL_INPUTS] {
        let mut events: [Option<TouchEvent>; MAX_DIGITAL_INPUTS] =
            [const { None }; MAX_DIGITAL_INPUTS];
        for (i, event) in events.iter_mut().enumerate().take(self.count) {
            if let Some(pin) = &mut self.pins[i] {
                let reading = measure_touch_async(pin).await;
                let touched = reading > self.pads[i].threshold;

                if touched != self.pads[i].was_touched {
                    self.pads[i].was_touched = touched;
                    *event = Some(TouchEvent {
                        #[allow(clippy::cast_possible_truncation)]
                        index: i as u8,
                        pressed: touched,
                    });
                }
            }
        }
        events
    }
}

/// Capacitive touch measurement using GPIO charge/discharge timing.
/// A finger touching the pad increases capacitance, which increases the
/// measured time.
///
/// The polarity is inverted to work around the RP2350-E9 erratum (which
/// causes pins to stick at ~2 V when pull-down is used after being driven
/// HIGH).  The pin is driven LOW, then switched to input with an internal
/// pull-up.  The ~50 kΩ pull-up charges the pad through the touch
/// capacitance, and we measure the time until the pin reads HIGH.  No
/// external resistor is required.
fn measure_touch_sync(pin: &mut Flex<'static>) -> u32 {
    pin.set_as_output();

    pin.set_low();

    cortex_m::asm::delay(1000);

    pin.set_pull(Pull::Up);

    let start = Instant::now();
    pin.set_as_input();

    let mut elapsed_us;
    loop {
        elapsed_us = start.elapsed().as_micros();

        let done = pin.is_high();

        if done || elapsed_us >= 500 {
            break;
        }
    }

    // Remove pull so it doesn't parasitically charge/discharge
    // the pad between measurements.
    pin.set_pull(Pull::None);

    #[allow(clippy::cast_possible_truncation)]
    {
        elapsed_us as u32
    }
}

async fn measure_touch_async(pin: &mut Flex<'static>) -> u32 {
    pin.set_as_output();

    pin.set_low();

    Timer::after_micros(10).await;

    pin.set_pull(Pull::Up);

    let start = Instant::now();
    pin.set_as_input();

    let mut elapsed_us;
    loop {
        elapsed_us = start.elapsed().as_micros();

        let done = pin.is_high();

        if done || elapsed_us >= 500 {
            break;
        }
    }

    pin.set_pull(Pull::None);

    #[allow(clippy::cast_possible_truncation)]
    {
        elapsed_us as u32
    }
}

const LIS3DH_ADDR: u8 = 0x19;
const REG_CTRL1: u8 = 0x20;
const REG_CTRL3: u8 = 0x22;
const REG_CTRL4: u8 = 0x23;
const REG_CLICK_CFG: u8 = 0x38;
const REG_CLICK_SRC: u8 = 0x39;
const REG_CLICK_THS: u8 = 0x3A;
const REG_TIME_LIMIT: u8 = 0x3B;
const REG_TIME_LATENCY: u8 = 0x3C;
const REG_OUT_X_L: u8 = 0x28;

pub struct AccelReading {
    pub x_cc: Option<u8>,
    pub y_cc: Option<u8>,
    pub tapped: bool,
}

pub struct Accelerometer<'d> {
    i2c: I2c<'d, I2C1, i2c::Async>,
    x_smoothed: f32,
    y_smoothed: f32,
    last_x_cc: u8,
    last_y_cc: u8,
    dead_zone: f32,
    smoothing: f32,
    last_poll: Instant,
    error_count: u8,
    pub available: bool,
}

impl<'d> Accelerometer<'d> {
    pub async fn new(
        mut i2c: I2c<'d, I2C1, i2c::Async>,
        dead_zone_tenths: u8,
        smoothing_pct: u8,
    ) -> Self {
        let available = Self::init(&mut i2c).await.is_ok();
        if !available {
            defmt::warn!("LIS3DH init failed");
        }
        Self {
            i2c,
            x_smoothed: 0.0,
            y_smoothed: 0.0,
            last_x_cc: 64,
            last_y_cc: 64,
            dead_zone: f32::from(dead_zone_tenths) / 10.0,
            smoothing: f32::from(smoothing_pct) / 100.0,
            last_poll: Instant::now(),
            error_count: 0,
            available,
        }
    }

    pub fn update_params(&mut self, dead_zone_tenths: u8, smoothing_pct: u8) {
        self.dead_zone = f32::from(dead_zone_tenths) / 10.0;
        self.smoothing = f32::from(smoothing_pct) / 100.0;
    }

    pub async fn poll(&mut self) -> AccelReading {
        let now = Instant::now();
        if now.duration_since(self.last_poll).as_millis() < 20 || !self.available {
            return AccelReading {
                x_cc: None,
                y_cc: None,
                tapped: false,
            };
        }
        self.last_poll = now;

        // Read 6 bytes: X, Y, Z as 16-bit signed LE (auto-increment via 0x80 bit)
        let mut buf = [0u8; 6];
        if self
            .i2c
            .write_read_async(LIS3DH_ADDR, [REG_OUT_X_L | 0x80], &mut buf)
            .await
            .is_err()
        {
            self.error_count += 1;
            if self.error_count > 10 {
                self.available = false;
                defmt::error!("LIS3DH disabled after repeated I2C failures");
            }
            return AccelReading {
                x_cc: None,
                y_cc: None,
                tapped: false,
            };
        }
        self.error_count = 0;

        let x_raw = f32::from(i16::from_le_bytes([buf[0], buf[1]]));
        let y_raw = f32::from(i16::from_le_bytes([buf[2], buf[3]]));

        let scale = 8.0 * 9.81 / 32768.0;
        let x_ms2 = x_raw * scale;
        let y_ms2 = y_raw * scale;

        self.x_smoothed = self.smoothing * x_ms2 + (1.0 - self.smoothing) * self.x_smoothed;
        self.y_smoothed = self.smoothing * y_ms2 + (1.0 - self.smoothing) * self.y_smoothed;

        let x_cc = self.axis_to_cc(self.x_smoothed);
        let y_cc = self.axis_to_cc(self.y_smoothed);

        let x_changed = if x_cc.abs_diff(self.last_x_cc) >= 1 {
            self.last_x_cc = x_cc;
            Some(x_cc)
        } else {
            None
        };

        let y_changed = if y_cc.abs_diff(self.last_y_cc) >= 1 {
            self.last_y_cc = y_cc;
            Some(y_cc)
        } else {
            None
        };

        let mut click_src = [0u8; 1];
        let tapped = if self
            .i2c
            .write_read_async(LIS3DH_ADDR, [REG_CLICK_SRC], &mut click_src)
            .await
            .is_ok()
        {
            click_src[0] & 0x10 != 0
        } else {
            false
        };

        AccelReading {
            x_cc: x_changed,
            y_cc: y_changed,
            tapped,
        }
    }

    pub const fn current_x_cc(&self) -> u8 {
        self.last_x_cc
    }

    pub const fn current_y_cc(&self) -> u8 {
        self.last_y_cc
    }

    async fn init(i2c: &mut I2c<'_, I2C1, i2c::Async>) -> Result<(), i2c::Error> {
        // 100Hz ODR, all axes enabled
        i2c.write_async(LIS3DH_ADDR, [REG_CTRL1, 0x57]).await?;
        // Route click interrupt to INT1
        i2c.write_async(LIS3DH_ADDR, [REG_CTRL3, 0x80]).await?;
        // +/-8g full scale, BDU enabled, high-resolution output
        i2c.write_async(LIS3DH_ADDR, [REG_CTRL4, 0xA8]).await?;
        // Single-click on Z-axis
        i2c.write_async(LIS3DH_ADDR, [REG_CLICK_CFG, 0x10]).await?;
        // Click threshold ~1.5g (8g/128 * 24)
        i2c.write_async(LIS3DH_ADDR, [REG_CLICK_THS, 24]).await?;
        i2c.write_async(LIS3DH_ADDR, [REG_TIME_LIMIT, 10]).await?;
        // Latch click interrupt long enough for the 20ms polling loop
        // to catch it (20 * 10ms = 200ms @ 100Hz ODR).
        i2c.write_async(LIS3DH_ADDR, [REG_TIME_LATENCY, 20]).await?;
        Ok(())
    }

    fn axis_to_cc(&self, value: f32) -> u8 {
        let v = if value.abs() < self.dead_zone {
            0.0
        } else {
            value
        };
        let normalized = (v / 9.81).clamp(-1.0, 1.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            ((normalized + 1.0) * 63.5) as u8
        }
    }
}

/// Create an ADC channel from a GPIO pin number (26, 27, or 28).
///
/// # Safety
/// Caller must ensure the pin is not in use elsewhere.
pub unsafe fn adc_channel_from_gpio(gpio: u8) -> Option<Channel<'static>> {
    match gpio {
        26 => {
            let pin = embassy_rp::peripherals::PIN_26::steal();
            Some(Channel::new_pin(pin, Pull::None))
        }
        27 => {
            let pin = embassy_rp::peripherals::PIN_27::steal();
            Some(Channel::new_pin(pin, Pull::None))
        }
        28 => {
            let pin = embassy_rp::peripherals::PIN_28::steal();
            Some(Channel::new_pin(pin, Pull::None))
        }
        _ => None,
    }
}
