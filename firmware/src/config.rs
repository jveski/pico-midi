#[cfg(target_os = "none")]
use defmt::Format;
#[cfg(target_os = "none")]
use embassy_rp::flash::{self, Flash};
#[cfg(target_os = "none")]
use embassy_rp::peripherals::FLASH;
use serde::{Deserialize, Serialize};

pub const MAGIC: u32 = 0x4D49_4449; // "MIDI"
pub const VERSION: u8 = 7;
pub const MAX_DIGITAL_INPUTS: usize = 21;
pub const MAX_ANALOG_INPUTS: usize = 3;
pub const MAX_EXPR: usize = 16;
pub const SECTOR_SIZE: usize = 4096;
#[cfg(target_os = "none")]
#[allow(clippy::cast_possible_truncation)]
pub const CONFIG_OFFSET: u32 = (FLASH_SIZE - SECTOR_SIZE) as u32;
const HEADER_SIZE: usize = 5;

#[cfg(target_os = "none")]
pub const FLASH_SIZE: usize = 4 * 1024 * 1024;

/// All GPIOs 0-22 except GP2 (I2C SDA), GP3 (I2C SCL), GP25 (LED).
pub const DIGITAL_PINS: [u8; MAX_DIGITAL_INPUTS] = [
    0, 1, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
];

pub const ANALOG_PINS: [u8; MAX_ANALOG_INPUTS] = [26, 27, 28];

pub fn is_valid_digital_pin(gpio: u8) -> bool {
    DIGITAL_PINS.contains(&gpio)
}

pub fn is_valid_analog_pin(gpio: u8) -> bool {
    ANALOG_PINS.contains(&gpio)
}

/// A compact bytecode expression (up to `MAX_EXPR` bytes).
/// When `len == 0` the static value is used instead.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(target_os = "none", derive(Format))]
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

/// Shared interface for inputs that produce MIDI notes (buttons and touch pads).
pub trait NoteInput {
    fn note(&self) -> u8;
    fn velocity(&self) -> u8;
    fn note_expr(&self) -> &Expr;
    fn velocity_expr(&self) -> &Expr;
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(target_os = "none", derive(Format))]
pub struct ButtonDef {
    pub pin: u8,
    pub note: u8,
    pub velocity: u8,
    pub note_expr: Expr,
    pub velocity_expr: Expr,
}

impl NoteInput for ButtonDef {
    fn note(&self) -> u8 {
        self.note
    }
    fn velocity(&self) -> u8 {
        self.velocity
    }
    fn note_expr(&self) -> &Expr {
        &self.note_expr
    }
    fn velocity_expr(&self) -> &Expr {
        &self.velocity_expr
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(target_os = "none", derive(Format))]
pub struct TouchPadDef {
    pub pin: u8,
    pub note: u8,
    pub velocity: u8,
    /// Percentage above the calibrated baseline (e.g. 33 = 33%).
    pub threshold_pct: u8,
    pub note_expr: Expr,
    pub velocity_expr: Expr,
}

impl NoteInput for TouchPadDef {
    fn note(&self) -> u8 {
        self.note
    }
    fn velocity(&self) -> u8 {
        self.velocity
    }
    fn note_expr(&self) -> &Expr {
        &self.note_expr
    }
    fn velocity_expr(&self) -> &Expr {
        &self.velocity_expr
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(target_os = "none", derive(Format))]
pub struct PotDef {
    pub pin: u8,
    pub cc: u8,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(target_os = "none", derive(Format))]
pub struct LdrDef {
    pub pin: u8,
    pub cc: u8,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(target_os = "none", derive(Format))]
pub struct AccelConfig {
    pub enabled: bool,
    pub x_cc: u8,
    pub y_cc: u8,
    pub tap_note: u8,
    pub tap_velocity: u8,
    /// Tenths of m/s^2 (e.g. 13 = 1.3 m/s^2).
    pub dead_zone_tenths: u8,
    /// Alpha * 100 (e.g. 25 = 0.25).
    pub smoothing_pct: u8,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(target_os = "none", derive(Format))]
pub struct Config {
    pub midi_channel: u8,
    pub num_buttons: u8,
    pub buttons: [ButtonDef; MAX_DIGITAL_INPUTS],
    pub num_touch_pads: u8,
    pub touch_pads: [TouchPadDef; MAX_DIGITAL_INPUTS],
    pub num_pots: u8,
    pub pots: [PotDef; MAX_ANALOG_INPUTS],
    pub ldr_enabled: bool,
    pub ldr: LdrDef,
    pub accel: AccelConfig,
}

impl Config {
    pub fn active_buttons(&self) -> &[ButtonDef] {
        let n = (self.num_buttons as usize).min(MAX_DIGITAL_INPUTS);
        &self.buttons[..n]
    }

    pub fn active_touch_pads(&self) -> &[TouchPadDef] {
        let n = (self.num_touch_pads as usize).min(MAX_DIGITAL_INPUTS);
        &self.touch_pads[..n]
    }

    pub fn active_pots(&self) -> &[PotDef] {
        let n = (self.num_pots as usize).min(MAX_ANALOG_INPUTS);
        &self.pots[..n]
    }

    pub fn validate(&self) -> bool {
        let mut used = [false; 30];

        for b in self.active_buttons() {
            if !is_valid_digital_pin(b.pin) {
                return false;
            }
            if used[b.pin as usize] {
                return false;
            }
            used[b.pin as usize] = true;
        }

        for t in self.active_touch_pads() {
            if !is_valid_digital_pin(t.pin) {
                return false;
            }
            if used[t.pin as usize] {
                return false;
            }
            used[t.pin as usize] = true;
        }

        for p in self.active_pots() {
            if !is_valid_analog_pin(p.pin) {
                return false;
            }
            if used[p.pin as usize] {
                return false;
            }
            used[p.pin as usize] = true;
        }

        if self.ldr_enabled {
            if !is_valid_analog_pin(self.ldr.pin) {
                return false;
            }
            if used[self.ldr.pin as usize] {
                return false;
            }
        }

        true
    }

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

const fn default_button(pin: u8, note: u8, velocity: u8) -> ButtonDef {
    ButtonDef {
        pin,
        note,
        velocity,
        note_expr: Expr::empty(),
        velocity_expr: Expr::empty(),
    }
}

const fn default_touch(pin: u8, note: u8, velocity: u8, threshold_pct: u8) -> TouchPadDef {
    TouchPadDef {
        pin,
        note,
        velocity,
        threshold_pct,
        note_expr: Expr::empty(),
        velocity_expr: Expr::empty(),
    }
}

const fn empty_button() -> ButtonDef {
    ButtonDef {
        pin: 0,
        note: 60,
        velocity: 100,
        note_expr: Expr::empty(),
        velocity_expr: Expr::empty(),
    }
}

const fn empty_touch() -> TouchPadDef {
    TouchPadDef {
        pin: 0,
        note: 48,
        velocity: 100,
        threshold_pct: 33,
        note_expr: Expr::empty(),
        velocity_expr: Expr::empty(),
    }
}

const fn empty_pot() -> PotDef {
    PotDef { pin: 0, cc: 0 }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            midi_channel: 0,
            num_buttons: 1,
            buttons: {
                let mut arr = [empty_button(); MAX_DIGITAL_INPUTS];
                arr[0] = default_button(0, 60, 100);
                arr
            },
            num_touch_pads: 1,
            touch_pads: {
                let mut arr = [empty_touch(); MAX_DIGITAL_INPUTS];
                arr[0] = default_touch(6, 48, 100, 33);
                arr
            },
            num_pots: 1,
            pots: {
                let mut arr = [empty_pot(); MAX_ANALOG_INPUTS];
                arr[0] = PotDef { pin: 26, cc: 7 };
                arr
            },
            ldr_enabled: true,
            ldr: LdrDef { pin: 28, cc: 74 },
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

#[cfg(target_os = "none")]
pub fn save_config(
    flash: &mut Flash<'static, FLASH, flash::Blocking, { FLASH_SIZE }>,
    config: &Config,
) -> bool {
    let mut sector = [0xFFu8; SECTOR_SIZE];
    let n = config.encode(&mut sector);
    if n == 0 {
        return false;
    }

    let offset = CONFIG_OFFSET;
    #[allow(clippy::cast_possible_truncation)]
    if flash
        .blocking_erase(offset, offset + SECTOR_SIZE as u32)
        .is_err()
    {
        defmt::error!("flash erase failed");
        return false;
    }
    if flash.blocking_write(offset, &sector).is_err() {
        defmt::error!("flash write failed");
        return false;
    }
    defmt::info!("config saved to flash ({} bytes)", n);
    true
}

#[cfg(target_os = "none")]
pub fn load_config(
    flash: &mut Flash<'static, FLASH, flash::Blocking, { FLASH_SIZE }>,
) -> Option<Config> {
    let mut buf = [0u8; SECTOR_SIZE];
    if flash.blocking_read(CONFIG_OFFSET, &mut buf).is_err() {
        defmt::warn!("flash read failed");
        return None;
    }
    Config::decode(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        assert!(Config::default().validate());
    }

    #[test]
    fn validate_rejects_bad_pins() {
        // Invalid pin (GP25 = LED)
        let mut cfg = Config::default();
        cfg.buttons[0].pin = 25;
        assert!(!cfg.validate());

        // Duplicate pin within buttons
        let mut cfg = Config::default();
        cfg.num_buttons = 2;
        cfg.buttons[0].pin = 0;
        cfg.buttons[1].pin = 0;
        assert!(!cfg.validate());

        // Same pin used by button and touch pad
        let mut cfg = Config::default();
        cfg.buttons[0].pin = 6;
        cfg.touch_pads[0].pin = 6;
        assert!(!cfg.validate());
    }

    #[test]
    fn encode_decode_round_trip() {
        let cfg = Config::default();
        let mut buf = [0u8; SECTOR_SIZE];
        let n = cfg.encode(&mut buf);
        assert!(n > HEADER_SIZE);
        let decoded = Config::decode(&buf).expect("decode failed");
        assert_eq!(decoded.midi_channel, cfg.midi_channel);
        assert_eq!(decoded.num_buttons, cfg.num_buttons);
        assert_eq!(decoded.buttons[0].pin, cfg.buttons[0].pin);
        assert_eq!(decoded.buttons[0].note, cfg.buttons[0].note);
        assert_eq!(decoded.num_pots, cfg.num_pots);
        assert_eq!(decoded.pots[0].cc, cfg.pots[0].cc);
    }

    #[test]
    fn decode_rejects_bad_header() {
        // Wrong magic
        let mut buf = [0u8; 64];
        buf[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        buf[4] = VERSION;
        assert!(Config::decode(&buf).is_none());

        // Wrong version
        let mut buf = [0u8; 64];
        buf[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        buf[4] = VERSION.wrapping_add(1);
        assert!(Config::decode(&buf).is_none());

        // Too short
        assert!(Config::decode(&[0u8; 4]).is_none());
    }
}
