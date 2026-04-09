use embassy_rp::adc::{self, Adc, Channel};
use embassy_rp::gpio::{Flex, Input, Pull};
use embassy_rp::i2c::{self, I2c};
use embassy_rp::peripherals::I2C0;
use embassy_time::{Instant, Timer};

const DEBOUNCE_MS: u64 = 10;

pub struct Buttons<const N: usize> {
    pins: [Input<'static>; N],
    prev: [bool; N],
    stable_since: [Instant; N],
}

pub struct ButtonEvent {
    pub index: u8,
    pub pressed: bool,
}

impl<const N: usize> Buttons<N> {
    pub fn new(pins: [Input<'static>; N]) -> Self {
        let prev = [false; N];
        let stable_since = [Instant::now(); N];
        Self {
            pins,
            prev,
            stable_since,
        }
    }

    /// Check all buttons. Returns up to N state-change events.
    pub fn poll(&mut self) -> [Option<ButtonEvent>; N] {
        let now = Instant::now();
        let mut events: [Option<ButtonEvent>; N] = [const { None }; N];
        for (i, event) in events.iter_mut().enumerate() {
            let pressed = self.pins[i].is_low();
            if pressed != self.prev[i]
                && now.duration_since(self.stable_since[i]).as_millis() >= DEBOUNCE_MS
            {
                self.prev[i] = pressed;
                self.stable_since[i] = now;
                *event = Some(ButtonEvent {
                    #[allow(clippy::cast_possible_truncation)] // N <= 8
                    index: i as u8,
                    pressed,
                });
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

        // Convert 12-bit (0-4095) to 7-bit (0-127)
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // smoothed is non-negative, result clamped to 127
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

    /// Return the current smoothed CC value (0-127), or 0 if never polled.
    pub fn current_cc(&self) -> u8 {
        self.last_cc.unwrap_or(0)
    }
}

/// Simple capacitive touch using GPIO charge/discharge timing.
/// Charge the pin high, then switch to input with pull-down and count
/// how long it takes to discharge. A finger increases capacitance.
struct TouchPad {
    threshold: u32,
    was_touched: bool,
}

pub struct TouchPads<const N: usize> {
    pads: [TouchPad; N],
}

pub struct TouchEvent {
    pub index: u8,
    pub pressed: bool,
}

fn measure_touch_sync(pin: &mut Flex<'static>) -> u32 {
    pin.set_as_output();
    pin.set_high();
    cortex_m::asm::delay(1000);

    pin.set_as_input();
    pin.set_pull(Pull::Down);

    let start = Instant::now();
    let mut elapsed_us;
    loop {
        elapsed_us = start.elapsed().as_micros();
        if !pin.is_high() || elapsed_us >= 500 {
            break;
        }
    }
    #[allow(clippy::cast_possible_truncation)]
    {
        elapsed_us as u32
    }
}

async fn measure_touch_async(pin: &mut Flex<'static>) -> u32 {
    pin.set_as_output();
    pin.set_high();
    Timer::after_micros(10).await;

    pin.set_as_input();
    pin.set_pull(Pull::Down);

    let start = Instant::now();
    let mut elapsed_us;
    loop {
        elapsed_us = start.elapsed().as_micros();
        if !pin.is_high() || elapsed_us >= 500 {
            break;
        }
    }
    #[allow(clippy::cast_possible_truncation)]
    {
        elapsed_us as u32
    }
}

impl<const N: usize> TouchPads<N> {
    /// Initialize touch pads, measuring baseline capacitance.
    /// Each pad's threshold is `baseline + max(baseline * threshold_pcts[i] / 100, 2)`.
    pub fn new(pins: &mut [Flex<'static>; N], threshold_pcts: &[u8; N]) -> Self {
        let pads: [TouchPad; N] = core::array::from_fn(|i| {
            let mut sum: u32 = 0;
            for _ in 0..8 {
                sum += measure_touch_sync(&mut pins[i]);
            }
            let baseline = sum / 8;
            let pct = u32::from(threshold_pcts[i]);
            let margin = (baseline * pct / 100).max(2);
            TouchPad {
                threshold: baseline + margin,
                was_touched: false,
            }
        });
        Self { pads }
    }

    /// Poll all touch pads. Returns up to N state-change events.
    pub async fn poll(&mut self, pins: &mut [Flex<'static>; N]) -> [Option<TouchEvent>; N] {
        let mut events: [Option<TouchEvent>; N] = [const { None }; N];
        for i in 0..N {
            let reading = measure_touch_async(&mut pins[i]).await;
            let touched = reading > self.pads[i].threshold;

            if touched != self.pads[i].was_touched {
                self.pads[i].was_touched = touched;
                events[i] = Some(TouchEvent {
                    #[allow(clippy::cast_possible_truncation)] // N <= 8
                    index: i as u8,
                    pressed: touched,
                });
            }
        }
        events
    }
}

const LIS3DH_ADDR: u8 = 0x19;
const REG_CTRL1: u8 = 0x20;
const REG_CTRL4: u8 = 0x23;
const REG_CLICK_CFG: u8 = 0x38;
const REG_CLICK_THS: u8 = 0x3A;
const REG_TIME_LIMIT: u8 = 0x3B;
const REG_CLICK_SRC: u8 = 0x39;
const REG_OUT_X_L: u8 = 0x28;

pub struct Accelerometer<'d> {
    i2c: I2c<'d, I2C0, i2c::Async>,
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

pub struct AccelReading {
    pub x_cc: Option<u8>,
    pub y_cc: Option<u8>,
    pub tapped: bool,
}

impl<'d> Accelerometer<'d> {
    pub async fn new(
        mut i2c: I2c<'d, I2C0, i2c::Async>,
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

    async fn init(i2c: &mut I2c<'_, I2C0, i2c::Async>) -> Result<(), i2c::Error> {
        // 100Hz ODR, all axes enabled
        i2c.write_async(LIS3DH_ADDR, [REG_CTRL1, 0x57]).await?;
        // ±8g full scale, BDU enabled, high-resolution output
        i2c.write_async(LIS3DH_ADDR, [REG_CTRL4, 0xA8]).await?;
        // Single-click on Z-axis
        i2c.write_async(LIS3DH_ADDR, [REG_CLICK_CFG, 0x10]).await?;
        // Click threshold ~1.5g (8g/128 * 24 ≈ 1.5g)
        i2c.write_async(LIS3DH_ADDR, [REG_CLICK_THS, 24]).await?;
        i2c.write_async(LIS3DH_ADDR, [REG_TIME_LIMIT, 10]).await?;
        Ok(())
    }

    fn axis_to_cc(&self, value: f32) -> u8 {
        let v = if value.abs() < self.dead_zone {
            0.0
        } else {
            value
        };
        let normalized = (v / 9.81).clamp(-1.0, 1.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Result is 0..=127
        {
            ((normalized + 1.0) * 63.5) as u8
        }
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

        // 8g range: scale = 8g * 9.81 / 32768
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
}
