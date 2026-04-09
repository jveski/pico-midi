use crate::config::{self, Config, SECTOR_SIZE};

use embassy_rp::flash::{self, Flash};
use embassy_rp::peripherals::FLASH;
use serde::{Deserialize, Serialize};

/// Commands sent from the host to the device.
#[derive(Deserialize)]
pub enum Request {
    /// Echo request.
    Ping,
    /// Query firmware version.
    Version,
    /// Read the current in-memory config.
    GetConfig,
    /// Replace the in-memory config.
    PutConfig(Config),
    /// Write the current config to flash.
    Save,
    /// Restore default config (in RAM).
    Reset,
}

/// Responses sent from the device to the host.
#[derive(Serialize)]
pub enum Response<'a> {
    /// Echo reply.
    Pong,
    /// Firmware version string.
    Version(&'a str),
    /// Current config.
    Config(Config),
    /// Command succeeded.
    Ok,
    /// Command failed with a reason.
    Error(&'a str),
    /// Live input monitor snapshot.
    Monitor(MonitorSnapshot),
}

#[derive(Serialize)]
pub struct MonitorSnapshot {
    pub buttons: [bool; config::MAX_BUTTONS],
    pub touch_pads: [bool; config::MAX_TOUCH_PADS],
    pub pots: [u8; config::MAX_POTS],
    pub ldr: u8,
    pub accel_x: u8,
    pub accel_y: u8,
    pub accel_tap: bool,
}

#[derive(PartialEq)]
pub enum Action {
    None,
    Save,
}

pub fn handle_frame(frame: &mut [u8], config: &mut Config, resp: &mut [u8]) -> (usize, Action) {
    let request = match postcard::from_bytes_cobs::<Request>(frame) {
        Ok(req) => req,
        Err(_) => {
            return encode_response(&Response::Error("bad frame"), resp, Action::None);
        }
    };

    match request {
        Request::Ping => encode_response(&Response::Pong, resp, Action::None),
        Request::Version => {
            encode_response(&Response::Version("midictrl 0.1.0"), resp, Action::None)
        }
        Request::GetConfig => encode_response(&Response::Config(*config), resp, Action::None),
        Request::PutConfig(new_config) => {
            *config = new_config;
            encode_response(&Response::Ok, resp, Action::None)
        }
        Request::Save => encode_response(&Response::Ok, resp, Action::Save),
        Request::Reset => {
            *config = Config::default();
            encode_response(&Response::Ok, resp, Action::None)
        }
    }
}

fn encode_response(response: &Response, buf: &mut [u8], action: Action) -> (usize, Action) {
    match postcard::to_slice_cobs(response, buf) {
        Ok(used) => {
            let n = used.len();
            // postcard::to_slice_cobs already appends the 0x00 sentinel
            (n, action)
        }
        Err(_) => {
            // Fallback: try to send a minimal error
            if let Ok(used) = postcard::to_slice_cobs(&Response::Error("encode error"), buf) {
                (used.len(), action)
            } else {
                (0, action)
            }
        }
    }
}

pub fn encode_monitor(snapshot: MonitorSnapshot, buf: &mut [u8]) -> usize {
    let resp = Response::Monitor(snapshot);
    match postcard::to_slice_cobs(&resp, buf) {
        Ok(used) => used.len(),
        Err(_) => 0,
    }
}

pub fn encode_error(msg: &str, buf: &mut [u8]) -> usize {
    match postcard::to_slice_cobs(&Response::Error(msg), buf) {
        Ok(used) => used.len(),
        Err(_) => 0,
    }
}

pub fn save_config(
    flash: &mut Flash<'static, FLASH, flash::Blocking, { config::FLASH_SIZE }>,
    config: &Config,
) -> bool {
    let mut sector = [0xFFu8; SECTOR_SIZE];
    let n = config.encode(&mut sector);
    if n == 0 {
        return false;
    }

    let offset = config::CONFIG_OFFSET;
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

pub fn load_config(
    flash: &mut Flash<'static, FLASH, flash::Blocking, { config::FLASH_SIZE }>,
) -> Option<Config> {
    let mut buf = [0u8; SECTOR_SIZE];
    if flash
        .blocking_read(config::CONFIG_OFFSET, &mut buf)
        .is_err()
    {
        defmt::warn!("flash read failed");
        return None;
    }
    Config::decode(&buf)
}
