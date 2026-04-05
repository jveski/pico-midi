//! Shared input state for real-time monitoring.
//!
//! The MIDI task writes the latest input values using atomic stores.
//! The serial task reads a snapshot for streaming to the host.
//! All fields are atomic so no locks are needed on single-core RP2040/RP2350.

use portable_atomic::{AtomicBool, AtomicU8, Ordering};

use crate::config;

/// Live input state shared between the MIDI polling task and serial monitor.
pub struct InputState {
    /// Button pressed states (up to MAX_BUTTONS).
    buttons: [AtomicBool; config::MAX_BUTTONS],
    /// Touch pad pressed states (up to MAX_TOUCH_PADS).
    touch_pads: [AtomicBool; config::MAX_TOUCH_PADS],
    /// Potentiometer CC values (up to MAX_POTS), 0-127.
    pots: [AtomicU8; config::MAX_POTS],
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
                AtomicBool::new(false), AtomicBool::new(false),
                AtomicBool::new(false), AtomicBool::new(false),
                AtomicBool::new(false), AtomicBool::new(false),
                AtomicBool::new(false), AtomicBool::new(false),
            ],
            touch_pads: [
                AtomicBool::new(false), AtomicBool::new(false),
                AtomicBool::new(false), AtomicBool::new(false),
                AtomicBool::new(false), AtomicBool::new(false),
                AtomicBool::new(false), AtomicBool::new(false),
            ],
            pots: [
                AtomicU8::new(0), AtomicU8::new(0),
                AtomicU8::new(0), AtomicU8::new(0),
            ],
            ldr: AtomicU8::new(0),
            accel_x: AtomicU8::new(64),
            accel_y: AtomicU8::new(64),
            accel_tap: AtomicBool::new(false),
        }
    }

    // ---- Writers (called from MIDI task) ----

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

    // ---- Reader (called from serial task) ----

    /// Format the current input state as a monitor line.
    ///
    /// Format: `M:b=01100000,t=11010000,p=64,127,l=42,ax=65,ay=58,at=0\n`
    /// - b=: button states (0/1 per button, MAX_BUTTONS chars)
    /// - t=: touch pad states (0/1 per pad, MAX_TOUCH_PADS chars)
    /// - p=: pot CC values, comma-separated
    /// - l=: LDR CC value
    /// - ax=: accelerometer X CC
    /// - ay=: accelerometer Y CC
    /// - at=: accelerometer tap (0 or 1, cleared on read)
    ///
    /// Returns the number of bytes written.
    pub fn format_snapshot(&self, buf: &mut [u8]) -> usize {
        let mut w = Writer::new(buf);

        // Buttons
        w.str("M:b=");
        for i in 0..config::MAX_BUTTONS {
            w.byte(if self.buttons[i].load(Ordering::Relaxed) { b'1' } else { b'0' });
        }

        // Touch pads
        w.str(",t=");
        for i in 0..config::MAX_TOUCH_PADS {
            w.byte(if self.touch_pads[i].load(Ordering::Relaxed) { b'1' } else { b'0' });
        }

        // Pots
        w.str(",p=");
        for i in 0..config::MAX_POTS {
            if i > 0 {
                w.byte(b':');
            }
            w.u8_dec(self.pots[i].load(Ordering::Relaxed));
        }

        // LDR
        w.str(",l=");
        w.u8_dec(self.ldr.load(Ordering::Relaxed));

        // Accelerometer
        w.str(",ax=");
        w.u8_dec(self.accel_x.load(Ordering::Relaxed));
        w.str(",ay=");
        w.u8_dec(self.accel_y.load(Ordering::Relaxed));
        w.str(",at=");
        // Swap tap flag: read and clear atomically
        let tapped = self.accel_tap.swap(false, Ordering::Relaxed);
        w.byte(if tapped { b'1' } else { b'0' });

        w.byte(b'\n');
        w.pos
    }
}

// Tiny no-alloc writer, same pattern as config.rs
struct Writer<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> Writer<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn byte(&mut self, b: u8) {
        if self.pos < self.buf.len() {
            self.buf[self.pos] = b;
            self.pos += 1;
        }
    }

    fn str(&mut self, s: &str) {
        for &b in s.as_bytes() {
            self.byte(b);
        }
    }

    fn u8_dec(&mut self, mut v: u8) {
        if v >= 100 {
            self.byte(b'0' + v / 100);
            v %= 100;
            self.byte(b'0' + v / 10);
            self.byte(b'0' + v % 10);
        } else if v >= 10 {
            self.byte(b'0' + v / 10);
            self.byte(b'0' + v % 10);
        } else {
            self.byte(b'0' + v);
        }
    }
}

// Safety: InputState uses only atomic types, safe to share across tasks.
unsafe impl Sync for InputState {}
