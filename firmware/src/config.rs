use defmt::Format;
use serde::{Deserialize, Serialize};

pub const MAGIC: u32 = 0x4D49_4449; // "MIDI"
pub const VERSION: u8 = 5;
pub const MAX_BUTTONS: usize = 8;
pub const MAX_TOUCH_PADS: usize = 8;
pub const MAX_POTS: usize = 4;
pub const MAX_ENCODERS: usize = 2;
pub const SECTOR_SIZE: usize = 4096;
pub const CONFIG_OFFSET: u32 = (FLASH_SIZE - SECTOR_SIZE) as u32;
const HEADER_SIZE: usize = 5;

#[cfg(feature = "rp2040")]
pub const FLASH_SIZE: usize = 2 * 1024 * 1024;
#[cfg(feature = "rp2350")]
pub const FLASH_SIZE: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Format, Serialize, Deserialize)]
pub struct ButtonDef {
    pub note: u8,
    pub velocity: u8,
}

#[derive(Clone, Copy, Format, Serialize, Deserialize)]
pub struct TouchPadDef {
    pub note: u8,
    pub velocity: u8,
    /// Touch threshold as a percentage above the calibrated baseline (e.g. 33 = 33%).
    pub threshold_pct: u8,
}

#[derive(Clone, Copy, Format, Serialize, Deserialize)]
pub struct PotDef {
    pub cc: u8,
}

#[derive(Clone, Copy, Format, Serialize, Deserialize)]
pub struct EncoderDef {
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
    pub num_encoders: u8,
    pub encoders: [EncoderDef; MAX_ENCODERS],
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
                ButtonDef {
                    note: 60,
                    velocity: 100,
                },
                ButtonDef {
                    note: 62,
                    velocity: 100,
                },
                ButtonDef {
                    note: 64,
                    velocity: 100,
                },
                ButtonDef {
                    note: 65,
                    velocity: 100,
                },
                ButtonDef {
                    note: 0,
                    velocity: 0,
                },
                ButtonDef {
                    note: 0,
                    velocity: 0,
                },
                ButtonDef {
                    note: 0,
                    velocity: 0,
                },
                ButtonDef {
                    note: 0,
                    velocity: 0,
                },
            ],
            num_touch_pads: 5,
            touch_pads: [
                TouchPadDef {
                    note: 72,
                    velocity: 100,
                    threshold_pct: 33,
                },
                TouchPadDef {
                    note: 74,
                    velocity: 100,
                    threshold_pct: 33,
                },
                TouchPadDef {
                    note: 76,
                    velocity: 100,
                    threshold_pct: 33,
                },
                TouchPadDef {
                    note: 77,
                    velocity: 100,
                    threshold_pct: 33,
                },
                TouchPadDef {
                    note: 79,
                    velocity: 100,
                    threshold_pct: 33,
                },
                TouchPadDef {
                    note: 0,
                    velocity: 0,
                    threshold_pct: 33,
                },
                TouchPadDef {
                    note: 0,
                    velocity: 0,
                    threshold_pct: 33,
                },
                TouchPadDef {
                    note: 0,
                    velocity: 0,
                    threshold_pct: 33,
                },
            ],
            num_pots: 2,
            pots: [
                PotDef { cc: 7 },
                PotDef { cc: 10 },
                PotDef { cc: 0 },
                PotDef { cc: 0 },
            ],
            num_encoders: 0,
            encoders: [EncoderDef { cc: 0 }, EncoderDef { cc: 0 }],
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

        match postcard::to_slice(self, &mut buf[HEADER_SIZE..]) {
            Ok(used) => HEADER_SIZE + used.len(),
            Err(_) => 0,
        }
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
