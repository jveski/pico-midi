//! Serial command protocol for configuration over USB CDC-ACM.
//!
//! Protocol (line-based, newline-terminated):
//!   PING          -> responds "PONG\n"
//!   VERSION       -> responds "midictrl 0.1.0\n"
//!   GET           -> responds with hex-encoded config bytes + "\n"
//!   PUT <hex>     -> decodes hex, replaces in-memory config, responds "OK\n"
//!   SAVE          -> writes current config to flash, responds "OK\n"
//!   RESET         -> restores defaults, responds "OK\n"
//!   REBOOT        -> responds "OK\n", then triggers software reset
//!
//! Monitor data (M: lines) is always streamed at ~50ms intervals when connected.
//! After SAVE, the device should be rebooted to apply hardware changes.

use crate::config::{self, Config, SECTOR_SIZE};

use embassy_rp::flash::{self, Flash};
use embassy_rp::peripherals::FLASH;

/// Process a single command line. Writes the response into `resp_buf`.
/// Returns (response_length, action).
pub fn handle_command(
    line: &[u8],
    config: &mut Config,
    resp: &mut [u8],
) -> (usize, Action) {
    let line = trim(line);

    if line == b"PING" {
        return (copy(resp, b"PONG\n"), Action::None);
    }

    if line == b"VERSION" {
        return (copy(resp, b"midictrl 0.1.0\n"), Action::None);
    }

    if line == b"GET" {
        let n = config.to_hex(resp);
        if n + 1 <= resp.len() {
            resp[n] = b'\n';
            return (n + 1, Action::None);
        }
        return (n, Action::None);
    }

    if line.len() > 4 && line.starts_with(b"PUT ") {
        let hex = &line[4..];
        if let Some(cfg) = Config::from_hex(hex) {
            *config = cfg;
            return (copy(resp, b"OK\n"), Action::None);
        } else {
            return (copy(resp, b"ERR bad config\n"), Action::None);
        }
    }

    if line == b"SAVE" {
        return (copy(resp, b"OK\n"), Action::Save);
    }

    if line == b"RESET" {
        *config = Config::default();
        return (copy(resp, b"OK\n"), Action::None);
    }

    if line == b"REBOOT" {
        return (copy(resp, b"OK\n"), Action::Reboot);
    }

    (copy(resp, b"ERR unknown command\n"), Action::None)
}

#[derive(PartialEq)]
pub enum Action {
    None,
    Save,
    Reboot,
}

/// Save config to flash. This erases and writes the last sector.
pub fn save_config(
    flash: &mut Flash<'static, FLASH, flash::Blocking, { config::FLASH_SIZE }>,
    config: &Config,
) -> bool {
    let mut sector = [0xFFu8; SECTOR_SIZE];
    let n = config.to_bytes(&mut sector);
    if n == 0 {
        return false;
    }

    let offset = config::CONFIG_OFFSET;
    if flash.blocking_erase(offset, offset + SECTOR_SIZE as u32).is_err() {
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

/// Load config from flash. Returns None if flash is blank or corrupt.
pub fn load_config(
    flash: &mut Flash<'static, FLASH, flash::Blocking, { config::FLASH_SIZE }>,
) -> Option<Config> {
    let mut buf = [0u8; SECTOR_SIZE];
    if flash.blocking_read(config::CONFIG_OFFSET, &mut buf).is_err() {
        defmt::warn!("flash read failed");
        return None;
    }
    Config::from_bytes(&buf)
}

// ---- helpers ----

fn trim(s: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = s.len();
    while start < end && matches!(s[start], b' ' | b'\t' | b'\r' | b'\n') {
        start += 1;
    }
    while end > start && matches!(s[end - 1], b' ' | b'\t' | b'\r' | b'\n') {
        end -= 1;
    }
    &s[start..end]
}

fn copy(dest: &mut [u8], src: &[u8]) -> usize {
    let n = src.len().min(dest.len());
    dest[..n].copy_from_slice(&src[..n]);
    n
}
