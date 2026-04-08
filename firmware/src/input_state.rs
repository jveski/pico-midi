use portable_atomic::{AtomicBool, AtomicU8, Ordering};

use crate::config;
use crate::serial::MonitorSnapshot;

/// Live input state shared between the MIDI polling task and serial monitor.
pub struct InputState {
    /// Button pressed states (up to MAX_BUTTONS).
    buttons: [AtomicBool; config::MAX_BUTTONS],
    /// Touch pad pressed states (up to MAX_TOUCH_PADS).
    touch_pads: [AtomicBool; config::MAX_TOUCH_PADS],
    /// Potentiometer CC values (up to MAX_POTS), 0-127.
    pots: [AtomicU8; config::MAX_POTS],
    /// Encoder CC values (up to MAX_ENCODERS), 0-127.
    encoders: [AtomicU8; config::MAX_ENCODERS],
    /// LDR CC value, 0-127.
    ldr: AtomicU8,
    /// Accelerometer X-axis CC value, 0-127.
    accel_x: AtomicU8,
    /// Accelerometer Y-axis CC value, 0-127.
    accel_y: AtomicU8,
    /// Accelerometer tap detected (cleared after read).
    accel_tap: AtomicBool,
}

impl InputState {
    pub const fn new() -> Self {
        Self {
            buttons: [
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
            ],
            touch_pads: [
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
            ],
            pots: [
                AtomicU8::new(0),
                AtomicU8::new(0),
                AtomicU8::new(0),
                AtomicU8::new(0),
            ],
            encoders: [AtomicU8::new(64), AtomicU8::new(64)],
            ldr: AtomicU8::new(0),
            accel_x: AtomicU8::new(64),
            accel_y: AtomicU8::new(64),
            accel_tap: AtomicBool::new(false),
        }
    }

    pub fn set_button(&self, index: u8, pressed: bool) {
        if (index as usize) < config::MAX_BUTTONS {
            self.buttons[index as usize].store(pressed, Ordering::Relaxed);
        }
    }

    pub fn set_touch(&self, index: u8, pressed: bool) {
        if (index as usize) < config::MAX_TOUCH_PADS {
            self.touch_pads[index as usize].store(pressed, Ordering::Relaxed);
        }
    }

    pub fn set_pot(&self, index: u8, value: u8) {
        if (index as usize) < config::MAX_POTS {
            self.pots[index as usize].store(value, Ordering::Relaxed);
        }
    }

    pub fn set_encoder(&self, index: u8, value: u8) {
        if (index as usize) < config::MAX_ENCODERS {
            self.encoders[index as usize].store(value, Ordering::Relaxed);
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

    /// Take a snapshot of the current input state.
    /// The accel_tap flag is cleared atomically on read.
    pub fn snapshot(&self) -> MonitorSnapshot {
        let mut buttons = [false; config::MAX_BUTTONS];
        for (i, button) in buttons.iter_mut().enumerate() {
            *button = self.buttons[i].load(Ordering::Relaxed);
        }

        let mut touch_pads = [false; config::MAX_TOUCH_PADS];
        for (i, touch_pad) in touch_pads.iter_mut().enumerate() {
            *touch_pad = self.touch_pads[i].load(Ordering::Relaxed);
        }

        let mut pots = [0u8; config::MAX_POTS];
        for (i, pot) in pots.iter_mut().enumerate() {
            *pot = self.pots[i].load(Ordering::Relaxed);
        }

        let mut encoders = [0u8; config::MAX_ENCODERS];
        for (i, enc) in encoders.iter_mut().enumerate() {
            *enc = self.encoders[i].load(Ordering::Relaxed);
        }

        MonitorSnapshot {
            buttons,
            touch_pads,
            pots,
            encoders,
            ldr: self.ldr.load(Ordering::Relaxed),
            accel_x: self.accel_x.load(Ordering::Relaxed),
            accel_y: self.accel_y.load(Ordering::Relaxed),
            accel_tap: self.accel_tap.swap(false, Ordering::Relaxed),
        }
    }
}

// Safety: InputState uses only atomic types, safe to share across tasks.
unsafe impl Sync for InputState {}
