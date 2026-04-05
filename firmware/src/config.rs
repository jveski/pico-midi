//! Flash-stored device configuration.
//!
//! Config is stored in the last 4K sector of flash. The layout is:
//! - 4 bytes: magic (0x4D494449 = "MIDI")
//! - 1 byte: version
//! - N bytes: postcard-encoded config fields
//! - Remaining: 0xFF (erased)

use defmt::Format;
use serde::{Deserialize, Serialize};

pub const MAGIC: u32 = 0x4D49_4449; // "MIDI"
pub const VERSION: u8 = 2;

pub const MAX_BUTTONS: usize = 8;
pub const MAX_TOUCH_PADS: usize = 8;
pub const MAX_POTS: usize = 4;

#[cfg(feature = "rp2040")]
pub const FLASH_SIZE: usize = 2 * 1024 * 1024;
#[cfg(feature = "rp2350")]
pub const FLASH_SIZE: usize = 4 * 1024 * 1024;

pub const SECTOR_SIZE: usize = 4096;
pub const CONFIG_OFFSET: u32 = (FLASH_SIZE - SECTOR_SIZE) as u32;

/// Header: 4-byte magic + 1-byte version.
const HEADER_SIZE: usize = 5;

/// A single button definition: GPIO pin number, MIDI note, velocity.
#[derive(Clone, Copy, Format, Serialize, Deserialize)]
pub struct ButtonDef {
    pub pin: u8,
    pub note: u8,
    pub velocity: u8,
}

/// A single touch pad definition: GPIO pin number, MIDI note, velocity.
#[derive(Clone, Copy, Format, Serialize, Deserialize)]
pub struct TouchPadDef {
    pub pin: u8,
    pub note: u8,
    pub velocity: u8,
}

/// A potentiometer/LDR definition: GPIO pin number (must be ADC-capable), CC number.
#[derive(Clone, Copy, Format, Serialize, Deserialize)]
pub struct PotDef {
    pub pin: u8,
    pub cc: u8,
}

/// Accelerometer configuration.
#[derive(Clone, Copy, Format, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Format, Serialize, Deserialize)]
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

impl Config {
    /// Serialize config into `buf` with a magic+version header.
    /// Returns the number of bytes written, or 0 on failure.
    pub fn encode(&self, buf: &mut [u8]) -> usize {
        if buf.len() < HEADER_SIZE {
            return 0;
        }
        buf[..4].copy_from_slice(&MAGIC.to_le_bytes());
        buf[4] = VERSION;

        match postcard::to_slice(self, &mut buf[HEADER_SIZE..]) {
            Ok(used) => HEADER_SIZE + used.len(),
            Err(_) => 0,
        }
    }

    /// Deserialize config from `buf`, validating the magic+version header.
    pub fn decode(buf: &[u8]) -> Option<Self> {
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
        postcard::from_bytes(&buf[HEADER_SIZE..]).ok()
    }

    /// Encode config as a hex string into the buffer. Returns bytes written.
    pub fn encode_hex(&self, buf: &mut [u8]) -> usize {
        let mut raw = [0u8; 128];
        let n = self.encode(&mut raw);
        hex_encode(&raw[..n], buf)
    }

    /// Decode a hex string and parse as config.
    pub fn decode_hex(hex: &[u8]) -> Option<Self> {
        let mut raw = [0u8; 128];
        let n = hex_decode(hex, &mut raw)?;
        Self::decode(&raw[..n])
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
    if !src.len().is_multiple_of(2) {
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
