use defmt::Format;
use serde::{Deserialize, Serialize};

pub const MAGIC: u32 = 0x4D49_4449; // "MIDI"
pub const VERSION: u8 = 5;
pub const MAX_BUTTONS: usize = 8;
pub const MAX_TOUCH_PADS: usize = 8;
pub const MAX_POTS: usize = 4;
pub const MAX_EXPR: usize = 16;
pub const SECTOR_SIZE: usize = 4096;
#[allow(clippy::cast_possible_truncation)] // Target is 32-bit ARM; usize == u32
pub const CONFIG_OFFSET: u32 = (FLASH_SIZE - SECTOR_SIZE) as u32;
const HEADER_SIZE: usize = 5;

#[cfg(feature = "rp2040")]
pub const FLASH_SIZE: usize = 2 * 1024 * 1024;
#[cfg(feature = "rp2350")]
pub const FLASH_SIZE: usize = 4 * 1024 * 1024;

/// A compact bytecode expression (up to `MAX_EXPR` bytes).
/// When `len == 0` the static value is used instead.
#[derive(Clone, Copy, Format, Serialize, Deserialize)]
pub struct Expr {
    pub len: u8,
    pub code: [u8; MAX_EXPR],
}

impl Expr {
    pub const fn empty() -> Self {
        Self {
            len: 0,
            code: [0; MAX_EXPR],
        }
    }
}

#[derive(Clone, Copy, Format, Serialize, Deserialize)]
pub struct ButtonDef {
    pub note: u8,
    pub velocity: u8,
    pub note_expr: Expr,
    pub velocity_expr: Expr,
}

#[derive(Clone, Copy, Format, Serialize, Deserialize)]
pub struct TouchPadDef {
    pub note: u8,
    pub velocity: u8,
    /// Touch threshold as a percentage above the calibrated baseline (e.g. 33 = 33%).
    pub threshold_pct: u8,
    pub note_expr: Expr,
    pub velocity_expr: Expr,
}

#[derive(Clone, Copy, Format, Serialize, Deserialize)]
pub struct PotDef {
    pub cc: u8,
}

#[derive(Clone, Copy, Format, Serialize, Deserialize)]
pub struct AccelConfig {
    pub enabled: bool,
    pub x_cc: u8,
    pub y_cc: u8,
    pub tap_note: u8,
    pub tap_velocity: u8,
    /// Dead zone in tenths of m/s^2 (e.g. 13 = 1.3 m/s^2)
    pub dead_zone_tenths: u8,
    /// Smoothing alpha * 100 (e.g. 25 = 0.25)
    pub smoothing_pct: u8,
}

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

const fn default_button(note: u8, velocity: u8) -> ButtonDef {
    ButtonDef {
        note,
        velocity,
        note_expr: Expr::empty(),
        velocity_expr: Expr::empty(),
    }
}

const fn default_touch(note: u8, velocity: u8, threshold_pct: u8) -> TouchPadDef {
    TouchPadDef {
        note,
        velocity,
        threshold_pct,
        note_expr: Expr::empty(),
        velocity_expr: Expr::empty(),
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            midi_channel: 0,
            num_buttons: 4,
            buttons: [
                default_button(60, 100),
                default_button(62, 100),
                default_button(64, 100),
                default_button(65, 100),
                default_button(0, 0),
                default_button(0, 0),
                default_button(0, 0),
                default_button(0, 0),
            ],
            num_touch_pads: 5,
            touch_pads: [
                default_touch(72, 100, 33),
                default_touch(74, 100, 33),
                default_touch(76, 100, 33),
                default_touch(77, 100, 33),
                default_touch(79, 100, 33),
                default_touch(0, 0, 33),
                default_touch(0, 0, 33),
                default_touch(0, 0, 33),
            ],
            num_pots: 2,
            pots: [
                PotDef { cc: 7 },
                PotDef { cc: 10 },
                PotDef { cc: 0 },
                PotDef { cc: 0 },
            ],
            ldr_enabled: true,
            ldr: PotDef { cc: 74 },
            accel: AccelConfig {
                enabled: true,
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
    pub fn encode(&self, buf: &mut [u8]) -> usize {
        if buf.len() < HEADER_SIZE {
            return 0;
        }
        buf[..4].copy_from_slice(&MAGIC.to_le_bytes());
        buf[4] = VERSION;

        postcard::to_slice(self, &mut buf[HEADER_SIZE..]).map_or(0, |used| HEADER_SIZE + used.len())
    }

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
}
