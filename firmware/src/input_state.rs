use portable_atomic::{AtomicBool, AtomicU8, Ordering};

use crate::config;
use crate::serial::MonitorSnapshot;

/// Live input state shared between the MIDI polling task and serial monitor.
pub struct InputState {
    /// Button pressed states (up to `MAX_DIGITAL_INPUTS`).
    buttons: [AtomicBool; config::MAX_DIGITAL_INPUTS],
    /// Touch pad pressed states (up to `MAX_DIGITAL_INPUTS`).
    touch_pads: [AtomicBool; config::MAX_DIGITAL_INPUTS],
    /// Potentiometer CC values (up to `MAX_ANALOG_INPUTS`), 0-127.
    pots: [AtomicU8; config::MAX_ANALOG_INPUTS],
    /// LDR CC value, 0-127.
    ldr: AtomicU8,
    /// Accelerometer X-axis CC value, 0-127.
    accel_x: AtomicU8,
    /// Accelerometer Y-axis CC value, 0-127.
    accel_y: AtomicU8,
    /// Accelerometer tap detected (cleared after read).
    accel_tap: AtomicBool,
}

// Helper to create a const array of AtomicBool::new(false).
macro_rules! atomic_bool_array {
    ($n:expr) => {{
        #[allow(clippy::declare_interior_mutable_const)]
        const INIT: AtomicBool = AtomicBool::new(false);
        [INIT; $n]
    }};
}

macro_rules! atomic_u8_array {
    ($n:expr, $val:expr) => {{
        #[allow(clippy::declare_interior_mutable_const)]
        const INIT: AtomicU8 = AtomicU8::new($val);
        [INIT; $n]
    }};
}

impl InputState {
    pub const fn new() -> Self {
        Self {
            buttons: atomic_bool_array!(config::MAX_DIGITAL_INPUTS),
            touch_pads: atomic_bool_array!(config::MAX_DIGITAL_INPUTS),
            pots: atomic_u8_array!(config::MAX_ANALOG_INPUTS, 0),
            ldr: AtomicU8::new(0),
            accel_x: AtomicU8::new(64),
            accel_y: AtomicU8::new(64),
            accel_tap: AtomicBool::new(false),
        }
    }

    pub fn set_button(&self, index: u8, pressed: bool) {
        if (index as usize) < config::MAX_DIGITAL_INPUTS {
            self.buttons[index as usize].store(pressed, Ordering::Relaxed);
        }
    }

    pub fn set_touch(&self, index: u8, pressed: bool) {
        if (index as usize) < config::MAX_DIGITAL_INPUTS {
            self.touch_pads[index as usize].store(pressed, Ordering::Relaxed);
        }
    }

    pub fn set_pot(&self, index: u8, value: u8) {
        if (index as usize) < config::MAX_ANALOG_INPUTS {
            self.pots[index as usize].store(value, Ordering::Relaxed);
        }
    }

    pub fn set_ldr(&self, value: u8) {
        self.ldr.store(value, Ordering::Relaxed);
    }

    pub fn set_accel_x(&self, value: u8) {
        self.accel_x.store(value, Ordering::Relaxed);
    }

    pub fn set_accel_y(&self, value: u8) {
        self.accel_y.store(value, Ordering::Relaxed);
    }

    pub fn set_accel_tap(&self) {
        self.accel_tap.store(true, Ordering::Relaxed);
    }

    /// Snapshot all pot values for expression evaluation.
    pub fn pots_snapshot(&self) -> [u8; config::MAX_ANALOG_INPUTS] {
        let mut v = [0u8; config::MAX_ANALOG_INPUTS];
        for (i, p) in v.iter_mut().enumerate() {
            *p = self.pots[i].load(Ordering::Relaxed);
        }
        v
    }

    pub fn ldr_value(&self) -> u8 {
        self.ldr.load(Ordering::Relaxed)
    }

    pub fn accel_x_value(&self) -> u8 {
        self.accel_x.load(Ordering::Relaxed)
    }

    pub fn accel_y_value(&self) -> u8 {
        self.accel_y.load(Ordering::Relaxed)
    }

    /// Take a snapshot of the current input state.
    /// The `accel_tap` flag is cleared atomically on read.
    pub fn snapshot(&self, num_buttons: u8, num_touch: u8, num_pots: u8) -> MonitorSnapshot {
        let nb = (num_buttons as usize).min(config::MAX_DIGITAL_INPUTS);
        let nt = (num_touch as usize).min(config::MAX_DIGITAL_INPUTS);
        let np = (num_pots as usize).min(config::MAX_ANALOG_INPUTS);

        let mut buttons = [false; config::MAX_DIGITAL_INPUTS];
        for (i, button) in buttons.iter_mut().take(nb).enumerate() {
            *button = self.buttons[i].load(Ordering::Relaxed);
        }

        let mut touch_pads = [false; config::MAX_DIGITAL_INPUTS];
        for (i, touch_pad) in touch_pads.iter_mut().take(nt).enumerate() {
            *touch_pad = self.touch_pads[i].load(Ordering::Relaxed);
        }

        let mut pots = [0u8; config::MAX_ANALOG_INPUTS];
        for (i, pot) in pots.iter_mut().take(np).enumerate() {
            *pot = self.pots[i].load(Ordering::Relaxed);
        }

        MonitorSnapshot {
            num_buttons: num_buttons.min(config::MAX_DIGITAL_INPUTS as u8),
            buttons,
            num_touch_pads: num_touch.min(config::MAX_DIGITAL_INPUTS as u8),
            touch_pads,
            num_pots: num_pots.min(config::MAX_ANALOG_INPUTS as u8),
            pots,
            ldr: self.ldr_value(),
            accel_x: self.accel_x_value(),
            accel_y: self.accel_y_value(),
            accel_tap: self.accel_tap.swap(false, Ordering::Relaxed),
        }
    }
}

// Safety: InputState uses only atomic types, safe to share across tasks.
unsafe impl Sync for InputState {}
