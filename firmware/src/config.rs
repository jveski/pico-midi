//! Flash-stored device configuration.
//!
//! Config is stored in the last 4K sector of flash. The layout is:
//! - 4 bytes: magic (0x4D494449 = "MIDI")
//! - 1 byte: version
//! - N bytes: config fields
//! - Remaining: 0xFF (erased)

use defmt::Format;

pub const MAGIC: u32 = 0x4D49_4449; // "MIDI"
pub const VERSION: u8 = 1;

pub const MAX_BUTTONS: usize = 8;
pub const MAX_TOUCH_PADS: usize = 8;
pub const MAX_POTS: usize = 4;

#[cfg(feature = "rp2040")]
pub const FLASH_SIZE: usize = 2 * 1024 * 1024;
#[cfg(feature = "rp2350")]
pub const FLASH_SIZE: usize = 4 * 1024 * 1024;

pub const SECTOR_SIZE: usize = 4096;
pub const CONFIG_OFFSET: u32 = (FLASH_SIZE - SECTOR_SIZE) as u32;

/// A single button definition: GPIO pin number, MIDI note, velocity.
#[derive(Clone, Copy, Format)]
pub struct ButtonDef {
    pub pin: u8,
    pub note: u8,
    pub velocity: u8,
}

/// A single touch pad definition: GPIO pin number, MIDI note, velocity.
#[derive(Clone, Copy, Format)]
pub struct TouchPadDef {
    pub pin: u8,
    pub note: u8,
    pub velocity: u8,
}

/// A potentiometer/LDR definition: GPIO pin number (must be ADC-capable), CC number.
#[derive(Clone, Copy, Format)]
pub struct PotDef {
    pub pin: u8,
    pub cc: u8,
}

/// Accelerometer configuration.
#[derive(Clone, Copy, Format)]
pub struct AccelConfig {
    pub enabled: bool,
    pub sda_pin: u8,
    pub scl_pin: u8,
    pub int_pin: u8,
    pub x_cc: u8,
    pub y_cc: u8,
    pub tap_note: u8,
    pub tap_velocity: u8,
    /// Dead zone in tenths of m/s^2 (e.g. 13 = 1.3 m/s^2)
    pub dead_zone_tenths: u8,
    /// Smoothing alpha * 100 (e.g. 25 = 0.25)
    pub smoothing_pct: u8,
}

/// Complete device configuration stored in flash.
#[derive(Clone, Copy, Format)]
pub struct Config {
    pub midi_channel: u8,
    pub num_buttons: u8,
    pub buttons: [ButtonDef; MAX_BUTTONS],
    pub num_touch_pads: u8,
    pub touch_pads: [TouchPadDef; MAX_TOUCH_PADS],
    pub num_pots: u8,
    pub pots: [PotDef; MAX_POTS],
    pub ldr: PotDef,
    pub ldr_enabled: bool,
    pub accel: AccelConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            midi_channel: 0,
            num_buttons: 4,
            buttons: [
                ButtonDef { pin: 2, note: 60, velocity: 100 },
                ButtonDef { pin: 3, note: 62, velocity: 100 },
                ButtonDef { pin: 4, note: 64, velocity: 100 },
                ButtonDef { pin: 5, note: 65, velocity: 100 },
                ButtonDef { pin: 0, note: 0, velocity: 0 },
                ButtonDef { pin: 0, note: 0, velocity: 0 },
                ButtonDef { pin: 0, note: 0, velocity: 0 },
                ButtonDef { pin: 0, note: 0, velocity: 0 },
            ],
            num_touch_pads: 5,
            touch_pads: [
                TouchPadDef { pin: 6, note: 72, velocity: 100 },
                TouchPadDef { pin: 7, note: 74, velocity: 100 },
                TouchPadDef { pin: 8, note: 76, velocity: 100 },
                TouchPadDef { pin: 9, note: 77, velocity: 100 },
                TouchPadDef { pin: 10, note: 79, velocity: 100 },
                TouchPadDef { pin: 0, note: 0, velocity: 0 },
                TouchPadDef { pin: 0, note: 0, velocity: 0 },
                TouchPadDef { pin: 0, note: 0, velocity: 0 },
            ],
            num_pots: 2,
            pots: [
                PotDef { pin: 26, cc: 7 },
                PotDef { pin: 27, cc: 10 },
                PotDef { pin: 0, cc: 0 },
                PotDef { pin: 0, cc: 0 },
            ],
            ldr_enabled: true,
            ldr: PotDef { pin: 28, cc: 74 },
            accel: AccelConfig {
                enabled: true,
                sda_pin: 0,
                scl_pin: 1,
                int_pin: 11,
                x_cc: 1,
                y_cc: 2,
                tap_note: 48,
                tap_velocity: 127,
                dead_zone_tenths: 13,
                smoothing_pct: 25,
            },
        }
    }
}

/// Serialization format:
/// [magic:4][version:1][midi_channel:1]
/// [num_buttons:1][buttons: num_buttons * 3]
/// [num_touch_pads:1][touch_pads: num_touch_pads * 3]
/// [num_pots:1][pots: num_pots * 2]
/// [ldr_enabled:1][ldr: 2]
/// [accel: 10]
const HEADER_SIZE: usize = 5; // magic + version

impl Config {
    pub fn to_bytes(&self, buf: &mut [u8]) -> usize {
        let mut i = 0;

        // Header
        buf[i..i + 4].copy_from_slice(&MAGIC.to_le_bytes());
        i += 4;
        buf[i] = VERSION;
        i += 1;

        // MIDI channel
        buf[i] = self.midi_channel;
        i += 1;

        // Buttons
        let nb = (self.num_buttons as usize).min(MAX_BUTTONS);
        buf[i] = nb as u8;
        i += 1;
        for j in 0..nb {
            buf[i] = self.buttons[j].pin;
            buf[i + 1] = self.buttons[j].note;
            buf[i + 2] = self.buttons[j].velocity;
            i += 3;
        }

        // Touch pads
        let nt = (self.num_touch_pads as usize).min(MAX_TOUCH_PADS);
        buf[i] = nt as u8;
        i += 1;
        for j in 0..nt {
            buf[i] = self.touch_pads[j].pin;
            buf[i + 1] = self.touch_pads[j].note;
            buf[i + 2] = self.touch_pads[j].velocity;
            i += 3;
        }

        // Pots
        let np = (self.num_pots as usize).min(MAX_POTS);
        buf[i] = np as u8;
        i += 1;
        for j in 0..np {
            buf[i] = self.pots[j].pin;
            buf[i + 1] = self.pots[j].cc;
            i += 2;
        }

        // LDR
        buf[i] = self.ldr_enabled as u8;
        i += 1;
        buf[i] = self.ldr.pin;
        buf[i + 1] = self.ldr.cc;
        i += 2;

        // Accelerometer
        buf[i] = self.accel.enabled as u8;
        buf[i + 1] = self.accel.sda_pin;
        buf[i + 2] = self.accel.scl_pin;
        buf[i + 3] = self.accel.int_pin;
        buf[i + 4] = self.accel.x_cc;
        buf[i + 5] = self.accel.y_cc;
        buf[i + 6] = self.accel.tap_note;
        buf[i + 7] = self.accel.tap_velocity;
        buf[i + 8] = self.accel.dead_zone_tenths;
        buf[i + 9] = self.accel.smoothing_pct;
        i += 10;

        i
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < HEADER_SIZE + 1 {
            return None;
        }

        let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if magic != MAGIC {
            return None;
        }
        if buf[4] != VERSION {
            return None;
        }

        let mut cfg = Config::default();
        let mut i = 5;

        // MIDI channel
        if i >= buf.len() { return None; }
        cfg.midi_channel = buf[i].min(15);
        i += 1;

        // Buttons
        if i >= buf.len() { return None; }
        let nb = (buf[i] as usize).min(MAX_BUTTONS);
        cfg.num_buttons = nb as u8;
        i += 1;
        for j in 0..nb {
            if i + 2 >= buf.len() { return None; }
            cfg.buttons[j] = ButtonDef {
                pin: buf[i],
                note: buf[i + 1].min(127),
                velocity: buf[i + 2].min(127).max(1),
            };
            i += 3;
        }

        // Touch pads
        if i >= buf.len() { return None; }
        let nt = (buf[i] as usize).min(MAX_TOUCH_PADS);
        cfg.num_touch_pads = nt as u8;
        i += 1;
        for j in 0..nt {
            if i + 2 >= buf.len() { return None; }
            cfg.touch_pads[j] = TouchPadDef {
                pin: buf[i],
                note: buf[i + 1].min(127),
                velocity: buf[i + 2].min(127).max(1),
            };
            i += 3;
        }

        // Pots
        if i >= buf.len() { return None; }
        let np = (buf[i] as usize).min(MAX_POTS);
        cfg.num_pots = np as u8;
        i += 1;
        for j in 0..np {
            if i + 1 >= buf.len() { return None; }
            cfg.pots[j] = PotDef {
                pin: buf[i],
                cc: buf[i + 1].min(127),
            };
            i += 2;
        }

        // LDR
        if i + 2 >= buf.len() { return None; }
        cfg.ldr_enabled = buf[i] != 0;
        cfg.ldr = PotDef {
            pin: buf[i + 1],
            cc: buf[i + 2].min(127),
        };
        i += 3;

        // Accelerometer
        if i + 9 >= buf.len() { return None; }
        cfg.accel = AccelConfig {
            enabled: buf[i] != 0,
            sda_pin: buf[i + 1],
            scl_pin: buf[i + 2],
            int_pin: buf[i + 3],
            x_cc: buf[i + 4].min(127),
            y_cc: buf[i + 5].min(127),
            tap_note: buf[i + 6].min(127),
            tap_velocity: buf[i + 7].min(127).max(1),
            dead_zone_tenths: buf[i + 8],
            smoothing_pct: buf[i + 9],
        };

        Some(cfg)
    }

    /// Encode config as a hex string into the buffer. Returns bytes written.
    pub fn to_hex(&self, buf: &mut [u8]) -> usize {
        let mut raw = [0u8; 128];
        let n = self.to_bytes(&mut raw);
        hex_encode(&raw[..n], buf)
    }

    /// Decode a hex string and parse as config.
    pub fn from_hex(hex: &[u8]) -> Option<Self> {
        let mut raw = [0u8; 128];
        let n = hex_decode(hex, &mut raw)?;
        Self::from_bytes(&raw[..n])
    }
}

// ---- Hex encode/decode ----

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

fn hex_encode(src: &[u8], dst: &mut [u8]) -> usize {
    let n = src.len().min(dst.len() / 2);
    for i in 0..n {
        dst[i * 2] = HEX_CHARS[(src[i] >> 4) as usize];
        dst[i * 2 + 1] = HEX_CHARS[(src[i] & 0x0F) as usize];
    }
    n * 2
}

fn hex_decode(src: &[u8], dst: &mut [u8]) -> Option<usize> {
    if src.len() % 2 != 0 {
        return None;
    }
    let n = (src.len() / 2).min(dst.len());
    for i in 0..n {
        let hi = hex_val(src[i * 2])?;
        let lo = hex_val(src[i * 2 + 1])?;
        dst[i] = (hi << 4) | lo;
    }
    Some(n)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
