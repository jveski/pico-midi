use cortex_m::peripheral::DWT;
use embassy_rp::adc::{self, Adc, Channel};
use embassy_rp::gpio::{Drive, Flex, Input, Pull};
use embassy_rp::i2c::{self, I2c};
use embassy_rp::peripherals::I2C1;
use embassy_time::Instant;

use crate::config::{AccelChip, MAX_DIGITAL_INPUTS};

const DEBOUNCE_MS: u64 = 10;

/// Minimum touch threshold in CPU cycles.  At 150 MHz this is ~2 µs, which
/// prevents false triggers on very-low-capacitance pads where the
/// percentage-based margin would be too small.
const MIN_THRESHOLD_CYCLES: u32 = 300;

/// ~75 000 cycles at 150 MHz ≈ 500 µs.  If a pad takes longer than this to
/// charge, the measurement returns the timeout value.
const TIMEOUT_CYCLES: u32 = 75_000;

/// Consecutive agreeing samples required to register a press.
const TOUCH_PRESS_COUNT: u8 = 3;
/// Consecutive agreeing samples required to register a release.
/// Higher than press because a false release (note stutter) is more
/// disruptive than a slightly delayed release.
const TOUCH_RELEASE_COUNT: u8 = 4;

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

        // Seed the filter with the first reading to avoid an audible
        // parameter sweep from zero to the actual value at startup.
        if self.last_cc.is_none() {
            self.smoothed = f32::from(raw);
        } else {
            self.smoothed = self.alpha * f32::from(raw) + (1.0 - self.alpha) * self.smoothed;
        }

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
    release_threshold: u32,
    filtered: u32,
    was_touched: bool,
    debounce_count: u8,
}

/// Per-pad telemetry exposed to the monitor for UI display.
#[derive(Clone, Copy, Default)]
pub struct TouchTelemetry {
    pub filtered: u32,
    pub baseline: u32,
    pub threshold: u32,
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
                    release_threshold: 0,
                    filtered: 0,
                    was_touched: false,
                    debounce_count: 0,
                }
            }; MAX_DIGITAL_INPUTS],
            pins: [const { None }; MAX_DIGITAL_INPUTS],
            count: 0,
        }
    }

    /// # Safety
    /// Caller must ensure the GPIO numbers are valid and not in use elsewhere.
    pub unsafe fn configure(&mut self, gpio_pins: &[u8], threshold_pcts: &[u8]) {
        // Drop all existing pin drivers and reset pad state (including
        // indices beyond the new count) to avoid stale `was_touched` state
        // causing spurious release events if the count is later increased.
        for pin in self.pins.iter_mut() {
            *pin = None;
        }
        for pad in self.pads.iter_mut() {
            *pad = TouchPad {
                baseline: 0,
                threshold: 0,
                release_threshold: 0,
                filtered: 0,
                was_touched: false,
                debounce_count: 0,
            };
        }
        self.count = gpio_pins.len().min(MAX_DIGITAL_INPUTS);

        for (i, &gpio) in gpio_pins.iter().take(self.count).enumerate() {
            let any = embassy_rp::gpio::AnyPin::steal(gpio);
            let mut flex = Flex::new(any);

            flex.set_schmitt(true);
            flex.set_drive_strength(Drive::_4mA);

            let mut sum: u32 = 0;
            for _ in 0..16 {
                sum += oversample_touch_sync(&mut flex);
            }
            let baseline = sum / 16;

            // Calibration warning: if the baseline saturates at the
            // timeout ceiling, the pad is likely shorted, disconnected,
            // or had a finger resting on it at startup.  Keep the pad
            // active so the user can see live telemetry in the
            // configurator and diagnose; touch detection simply will
            // not trigger because `filtered` cannot exceed `threshold`.
            if baseline >= TIMEOUT_CYCLES {
                defmt::warn!(
                    "touch pad GP{} calibration saturated (baseline={}); pad will not detect touches",
                    gpio,
                    baseline
                );
            }

            let pct = u32::from(threshold_pcts.get(i).copied().unwrap_or(25));
            let margin = (baseline * pct / 100).max(MIN_THRESHOLD_CYCLES);

            self.pads[i] = TouchPad {
                baseline,
                threshold: baseline + margin,
                release_threshold: baseline + margin * 60 / 100,
                filtered: baseline,
                was_touched: false,
                debounce_count: 0,
            };
            self.pins[i] = Some(flex);
        }
    }

    pub fn update_thresholds(&mut self, threshold_pcts: &[u8]) {
        for (i, pad) in self.pads.iter_mut().take(self.count).enumerate() {
            if self.pins[i].is_none() {
                continue; // Skip pads that failed calibration.
            }
            let pct = u32::from(threshold_pcts.get(i).copied().unwrap_or(25));
            let margin = (pad.baseline * pct / 100).max(MIN_THRESHOLD_CYCLES);
            pad.threshold = pad.baseline + margin;
            pad.release_threshold = pad.baseline + margin * 60 / 100;
        }
    }

    /// Return per-pad telemetry for the monitor snapshot.
    pub fn telemetry(&self) -> [TouchTelemetry; MAX_DIGITAL_INPUTS] {
        let mut out = [TouchTelemetry::default(); MAX_DIGITAL_INPUTS];
        for (i, pad) in self.pads.iter().take(self.count).enumerate() {
            if self.pins[i].is_some() {
                out[i] = TouchTelemetry {
                    filtered: pad.filtered,
                    baseline: pad.baseline,
                    threshold: pad.threshold,
                };
            }
        }
        out
    }

    pub async fn poll(&mut self) -> [Option<TouchEvent>; MAX_DIGITAL_INPUTS] {
        let mut events: [Option<TouchEvent>; MAX_DIGITAL_INPUTS] =
            [const { None }; MAX_DIGITAL_INPUTS];
        for (i, event) in events.iter_mut().enumerate().take(self.count) {
            if let Some(pin) = &mut self.pins[i] {
                let raw = oversample_touch_async(pin).await;
                // IIR / EMA filter: α ≈ 0.30 using fixed-point 77/256.
                // filtered = (77 * raw + 179 * filtered_prev) / 256
                let pad = &mut self.pads[i];
                pad.filtered = (77 * raw + 179 * pad.filtered) / 256;

                let touched = if pad.was_touched {
                    // Currently touched: only release when below release threshold
                    pad.filtered > pad.release_threshold
                } else {
                    // Currently released: only touch when above touch threshold
                    pad.filtered > pad.threshold
                };

                if touched != pad.was_touched {
                    pad.debounce_count = pad.debounce_count.saturating_add(1);
                    let target = if pad.was_touched {
                        TOUCH_RELEASE_COUNT
                    } else {
                        TOUCH_PRESS_COUNT
                    };
                    if pad.debounce_count >= target {
                        pad.was_touched = touched;
                        pad.debounce_count = 0;
                        *event = Some(TouchEvent {
                            #[allow(clippy::cast_possible_truncation)]
                            index: i as u8,
                            pressed: touched,
                        });
                    }
                } else {
                    pad.debounce_count = 0;
                }
            }
        }
        events
    }
}

/// Trimmed mean of 4 values: discard the minimum and maximum, average the
/// middle two.  Eliminates single-sample spikes without a full sort.
fn trimmed_mean_4(a: u32, b: u32, c: u32, d: u32) -> u32 {
    let min = a.min(b).min(c).min(d);
    let max = a.max(b).max(c).max(d);
    // Sum all four, subtract the extremes, divide by 2.
    let sum = a + b + c + d - min - max;
    sum / 2
}

/// Take 4 synchronous measurements and return the trimmed mean.
fn oversample_touch_sync(pin: &mut Flex<'static>) -> u32 {
    let s0 = measure_touch_sync(pin);
    let s1 = measure_touch_sync(pin);
    let s2 = measure_touch_sync(pin);
    let s3 = measure_touch_sync(pin);
    trimmed_mean_4(s0, s1, s2, s3)
}

/// Take 4 async measurements and return the trimmed mean.
async fn oversample_touch_async(pin: &mut Flex<'static>) -> u32 {
    let s0 = measure_touch_async(pin).await;
    let s1 = measure_touch_async(pin).await;
    let s2 = measure_touch_async(pin).await;
    let s3 = measure_touch_async(pin).await;
    trimmed_mean_4(s0, s1, s2, s3)
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
///
/// Returns elapsed CPU cycles (not microseconds) for sub-microsecond
/// resolution.  At 150 MHz one cycle ≈ 6.7 ns.
fn measure_touch_sync(pin: &mut Flex<'static>) -> u32 {
    pin.set_as_output();

    pin.set_low();

    cortex_m::asm::delay(1000);

    pin.set_pull(Pull::Up);

    let start = DWT::cycle_count();
    pin.set_as_input();

    let mut elapsed;
    loop {
        elapsed = DWT::cycle_count().wrapping_sub(start);

        let done = pin.is_high();

        if done || elapsed >= TIMEOUT_CYCLES {
            break;
        }
    }

    // Remove pull so it doesn't parasitically charge/discharge
    // the pad between measurements.
    pin.set_pull(Pull::None);

    elapsed
}

async fn measure_touch_async(pin: &mut Flex<'static>) -> u32 {
    pin.set_as_output();

    pin.set_low();

    // Use the same busy-wait discharge time as the synchronous calibration
    // measurement to avoid a systematic baseline offset.
    cortex_m::asm::delay(1000);

    pin.set_pull(Pull::Up);

    let start = DWT::cycle_count();
    pin.set_as_input();

    let mut elapsed;
    loop {
        elapsed = DWT::cycle_count().wrapping_sub(start);

        let done = pin.is_high();

        if done || elapsed >= TIMEOUT_CYCLES {
            break;
        }
    }

    pin.set_pull(Pull::None);

    elapsed
}

// ---------------------------------------------------------------------------
// LIS3DH registers and address
// ---------------------------------------------------------------------------
const LIS3DH_ADDR: u8 = 0x19;
const LIS3DH_WHO_AM_I: u8 = 0x0F;
const LIS3DH_CTRL1: u8 = 0x20;
const LIS3DH_CTRL3: u8 = 0x22;
const LIS3DH_CTRL4: u8 = 0x23;
const LIS3DH_CLICK_CFG: u8 = 0x38;
const LIS3DH_CLICK_SRC: u8 = 0x39;
const LIS3DH_CLICK_THS: u8 = 0x3A;
const LIS3DH_TIME_LIMIT: u8 = 0x3B;
const LIS3DH_TIME_LATENCY: u8 = 0x3C;
const LIS3DH_OUT_X_L: u8 = 0x28;

// ---------------------------------------------------------------------------
// MPU6050 registers and address
// ---------------------------------------------------------------------------
const MPU6050_ADDR: u8 = 0x68;
const MPU6050_WHO_AM_I: u8 = 0x75;
const MPU6050_PWR_MGMT_1: u8 = 0x6B;
const MPU6050_SMPLRT_DIV: u8 = 0x19;
const MPU6050_DLPF_CFG: u8 = 0x1A;
const MPU6050_ACCEL_CFG: u8 = 0x1C;
const MPU6050_MOT_THR: u8 = 0x1F;
const MPU6050_MOT_DUR: u8 = 0x20;
const MPU6050_INT_ENABLE: u8 = 0x38;
const MPU6050_INT_STATUS: u8 = 0x3A;
const MPU6050_ACCEL_XOUT_H: u8 = 0x3B;

/// Which chip was actually detected at runtime.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DetectedChip {
    Lis3dh,
    Mpu6050,
}

pub struct AccelReading {
    pub x_cc: Option<u8>,
    pub y_cc: Option<u8>,
    pub tapped: bool,
}

pub struct Accelerometer<'d> {
    i2c: I2c<'d, I2C1, i2c::Async>,
    chip: Option<DetectedChip>,
    x_smoothed: f32,
    y_smoothed: f32,
    last_x_cc: u8,
    last_y_cc: u8,
    dead_zone: f32,
    smoothing: f32,
    last_poll: Instant,
    error_count: u8,
    /// `None` when the sensor is reachable; `Some(instant)` records when it
    /// was disabled so we can attempt periodic re-initialisation.
    disabled_at: Option<Instant>,
    /// The chip preference from config (used for re-init attempts).
    config_chip: AccelChip,
    /// Whether we have received the first accelerometer reading (used to
    /// seed the EMA filter and avoid a startup sweep).
    first_reading: bool,
    pub available: bool,
}

impl<'d> Accelerometer<'d> {
    pub async fn new(
        mut i2c: I2c<'d, I2C1, i2c::Async>,
        dead_zone_tenths: u8,
        smoothing_pct: u8,
        chip_pref: AccelChip,
    ) -> Self {
        let detected = Self::detect_and_init(&mut i2c, chip_pref).await;
        let available = detected.is_some();
        if !available {
            defmt::warn!("accelerometer init failed (pref={:?})", chip_pref);
        } else {
            match detected {
                Some(DetectedChip::Lis3dh) => defmt::info!("LIS3DH initialised"),
                Some(DetectedChip::Mpu6050) => defmt::info!("MPU6050 initialised"),
                None => {}
            }
        }
        Self {
            i2c,
            chip: detected,
            x_smoothed: 0.0,
            y_smoothed: 0.0,
            last_x_cc: 63, // matches axis_to_cc(0.0) to avoid startup jitter
            last_y_cc: 63,
            dead_zone: f32::from(dead_zone_tenths) / 10.0,
            smoothing: f32::from(smoothing_pct) / 100.0,
            last_poll: Instant::now(),
            error_count: 0,
            disabled_at: if available {
                None
            } else {
                Some(Instant::now())
            },
            config_chip: chip_pref,
            first_reading: true,
            available,
        }
    }

    pub fn update_params(&mut self, dead_zone_tenths: u8, smoothing_pct: u8, chip: AccelChip) {
        self.dead_zone = f32::from(dead_zone_tenths) / 10.0;
        self.smoothing = f32::from(smoothing_pct) / 100.0;

        // If the user changed the chip selector in the UI, mark the
        // accelerometer for immediate re-initialisation.  `Instant::MIN`
        // is the time epoch (tick 0); after >5 s of uptime the recovery
        // gate in `poll()` fires on the very next call.  Reset the
        // smoothing/state fields so the new chip's first reading seeds
        // the EMA cleanly and the change-detector sends a fresh CC.
        if chip != self.config_chip {
            self.config_chip = chip;
            self.chip = None;
            self.available = false;
            self.disabled_at = Some(Instant::MIN);
            self.first_reading = true;
            self.last_x_cc = 63;
            self.last_y_cc = 63;
            self.error_count = 0;
        }
    }

    pub async fn poll(&mut self) -> AccelReading {
        let now = Instant::now();

        // Attempt re-initialisation every 5 seconds after being disabled.
        if let Some(disabled) = self.disabled_at {
            if now.duration_since(disabled).as_secs() >= 5 {
                if let Some(det) = Self::detect_and_init(&mut self.i2c, self.config_chip).await {
                    defmt::info!("accelerometer recovered");
                    self.chip = Some(det);
                    self.available = true;
                    self.disabled_at = None;
                    self.error_count = 0;
                } else {
                    self.disabled_at = Some(now);
                }
            }
        }

        if now.duration_since(self.last_poll).as_millis() < 20 || !self.available {
            return AccelReading {
                x_cc: None,
                y_cc: None,
                tapped: false,
            };
        }
        self.last_poll = now;

        let det = match self.chip {
            Some(c) => c,
            None => {
                return AccelReading {
                    x_cc: None,
                    y_cc: None,
                    tapped: false,
                }
            }
        };

        // Read accelerometer data — chip-specific
        let (x_ms2, y_ms2) = match self.read_accel(det).await {
            Some(v) => v,
            None => {
                self.error_count += 1;
                if self.error_count > 10 {
                    self.available = false;
                    self.disabled_at = Some(Instant::now());
                    defmt::error!("accelerometer disabled after repeated I2C failures");
                }
                return AccelReading {
                    x_cc: None,
                    y_cc: None,
                    tapped: false,
                };
            }
        };
        self.error_count = 0;

        // Seed the filter with the first reading to avoid an audible
        // parameter sweep from zero to the actual value at startup.
        if self.first_reading {
            self.x_smoothed = x_ms2;
            self.y_smoothed = y_ms2;
            self.first_reading = false;
        } else {
            self.x_smoothed = self.smoothing * x_ms2 + (1.0 - self.smoothing) * self.x_smoothed;
            self.y_smoothed = self.smoothing * y_ms2 + (1.0 - self.smoothing) * self.y_smoothed;
        }

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

        let tapped = self.read_tap(det).await;

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

    // -----------------------------------------------------------------------
    // Detection & initialisation
    // -----------------------------------------------------------------------

    /// Probe the bus according to `pref` and initialise whichever chip is
    /// found.  Returns `None` when no supported chip responds.
    async fn detect_and_init(
        i2c: &mut I2c<'_, I2C1, i2c::Async>,
        pref: AccelChip,
    ) -> Option<DetectedChip> {
        match pref {
            AccelChip::Lis3dh => {
                if Self::init_lis3dh(i2c).await.is_ok() {
                    Some(DetectedChip::Lis3dh)
                } else {
                    None
                }
            }
            AccelChip::Mpu6050 => {
                if Self::init_mpu6050(i2c).await.is_ok() {
                    Some(DetectedChip::Mpu6050)
                } else {
                    None
                }
            }
            AccelChip::Auto => {
                // Try LIS3DH first (existing default), then MPU6050.
                if Self::init_lis3dh(i2c).await.is_ok() {
                    Some(DetectedChip::Lis3dh)
                } else if Self::init_mpu6050(i2c).await.is_ok() {
                    Some(DetectedChip::Mpu6050)
                } else {
                    None
                }
            }
        }
    }

    async fn init_lis3dh(i2c: &mut I2c<'_, I2C1, i2c::Async>) -> Result<(), i2c::Error> {
        // Verify WHO_AM_I (should return 0x33 for LIS3DH).
        let mut whoami = [0u8; 1];
        i2c.write_read_async(LIS3DH_ADDR, [LIS3DH_WHO_AM_I], &mut whoami)
            .await?;
        if whoami[0] != 0x33 {
            return Err(i2c::Error::Abort(i2c::AbortReason::NoAcknowledge));
        }

        // 100Hz ODR, all axes enabled
        i2c.write_async(LIS3DH_ADDR, [LIS3DH_CTRL1, 0x57]).await?;
        // Route click interrupt to INT1
        i2c.write_async(LIS3DH_ADDR, [LIS3DH_CTRL3, 0x80]).await?;
        // +/-8g full scale, BDU enabled, high-resolution output
        i2c.write_async(LIS3DH_ADDR, [LIS3DH_CTRL4, 0xA8]).await?;
        // Single-click on Z-axis
        i2c.write_async(LIS3DH_ADDR, [LIS3DH_CLICK_CFG, 0x10])
            .await?;
        // Click threshold ~1.5g (8g/128 * 24)
        i2c.write_async(LIS3DH_ADDR, [LIS3DH_CLICK_THS, 24]).await?;
        i2c.write_async(LIS3DH_ADDR, [LIS3DH_TIME_LIMIT, 10])
            .await?;
        // Latch click interrupt long enough for the 20ms polling loop
        // to catch it (20 * 10ms = 200ms @ 100Hz ODR).
        i2c.write_async(LIS3DH_ADDR, [LIS3DH_TIME_LATENCY, 20])
            .await?;
        Ok(())
    }

    async fn init_mpu6050(i2c: &mut I2c<'_, I2C1, i2c::Async>) -> Result<(), i2c::Error> {
        // Verify WHO_AM_I (should return 0x68 for a genuine MPU6050, but
        // some clones return 0x72 or 0x98 — accept any non-0xFF/0x00
        // response that we can actually read successfully).
        let mut whoami = [0u8; 1];
        i2c.write_read_async(MPU6050_ADDR, [MPU6050_WHO_AM_I], &mut whoami)
            .await?;
        if whoami[0] == 0x00 || whoami[0] == 0xFF {
            // Not a real device on the bus.
            return Err(i2c::Error::Abort(i2c::AbortReason::NoAcknowledge));
        }

        // Wake up from sleep (clear SLEEP bit)
        i2c.write_async(MPU6050_ADDR, [MPU6050_PWR_MGMT_1, 0x00])
            .await?;
        // Sample rate divider: 1kHz / (1+9) = 100Hz (matches LIS3DH)
        i2c.write_async(MPU6050_ADDR, [MPU6050_SMPLRT_DIV, 9])
            .await?;
        // DLPF config: ~44Hz bandwidth for smooth readings
        i2c.write_async(MPU6050_ADDR, [MPU6050_DLPF_CFG, 3]).await?;
        // Accel full-scale range: +/-8g (bits [4:3] = 0b10)
        i2c.write_async(MPU6050_ADDR, [MPU6050_ACCEL_CFG, 0x10])
            .await?;
        // Motion detection threshold: 40 * 2mg = 80mg (~0.08g).
        // Compared against acceleration change from the static position.
        // Moderate value that fires on taps but ignores slow tilting.
        i2c.write_async(MPU6050_ADDR, [MPU6050_MOT_THR, 40]).await?;
        // Motion detection duration: 2 samples at internal 1kHz (~2ms)
        i2c.write_async(MPU6050_ADDR, [MPU6050_MOT_DUR, 2]).await?;
        // Enable motion detection interrupt
        i2c.write_async(MPU6050_ADDR, [MPU6050_INT_ENABLE, 0x40])
            .await?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Data reading
    // -----------------------------------------------------------------------

    /// Read X/Y acceleration in m/s² from the detected chip.
    async fn read_accel(&mut self, det: DetectedChip) -> Option<(f32, f32)> {
        let mut buf = [0u8; 6];
        match det {
            DetectedChip::Lis3dh => {
                // Auto-increment via 0x80 bit
                self.i2c
                    .write_read_async(LIS3DH_ADDR, [LIS3DH_OUT_X_L | 0x80], &mut buf)
                    .await
                    .ok()?;
                let x_raw = f32::from(i16::from_le_bytes([buf[0], buf[1]]));
                let y_raw = f32::from(i16::from_le_bytes([buf[2], buf[3]]));
                // LIS3DH in high-res mode: 16-bit left-justified, +/-8g
                let scale = 8.0 * 9.81 / 32768.0;
                Some((x_raw * scale, y_raw * scale))
            }
            DetectedChip::Mpu6050 => {
                self.i2c
                    .write_read_async(MPU6050_ADDR, [MPU6050_ACCEL_XOUT_H], &mut buf)
                    .await
                    .ok()?;
                // MPU6050 data is big-endian: [XH, XL, YH, YL, ZH, ZL]
                let x_raw = f32::from(i16::from_be_bytes([buf[0], buf[1]]));
                let y_raw = f32::from(i16::from_be_bytes([buf[2], buf[3]]));
                // +/-8g: sensitivity = 4096 LSB/g
                let scale = 8.0 * 9.81 / 32768.0;
                Some((x_raw * scale, y_raw * scale))
            }
        }
    }

    /// Check for a tap event on the detected chip.
    async fn read_tap(&mut self, det: DetectedChip) -> bool {
        match det {
            DetectedChip::Lis3dh => {
                let mut click_src = [0u8; 1];
                if self
                    .i2c
                    .write_read_async(LIS3DH_ADDR, [LIS3DH_CLICK_SRC], &mut click_src)
                    .await
                    .is_ok()
                {
                    click_src[0] & 0x10 != 0
                } else {
                    false
                }
            }
            DetectedChip::Mpu6050 => {
                let mut status = [0u8; 1];
                if self
                    .i2c
                    .write_read_async(MPU6050_ADDR, [MPU6050_INT_STATUS], &mut status)
                    .await
                    .is_ok()
                {
                    // Bit 6: Motion Detection interrupt
                    status[0] & 0x40 != 0
                } else {
                    false
                }
            }
        }
    }

    fn axis_to_cc(&self, value: f32) -> u8 {
        // Subtract dead zone so the output transitions smoothly from 0
        // at the boundary instead of jumping discontinuously.
        let v = if value.abs() < self.dead_zone {
            0.0
        } else {
            value - value.signum() * self.dead_zone
        };
        let range = 9.81 - self.dead_zone;
        let normalized = if range > 0.0 {
            (v / range).clamp(-1.0, 1.0)
        } else {
            0.0
        };
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
