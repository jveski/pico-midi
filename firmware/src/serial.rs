use core::cell::RefCell;

use embassy_futures::select::{select, Either};
use embassy_rp::flash;
use embassy_rp::peripherals::{FLASH, USB};
use embassy_rp::usb::Driver;
use embassy_time::{Duration, Instant, Timer};
use embassy_usb::class::cdc_acm::CdcAcmClass;
use embassy_usb::driver::EndpointError;
use serde::{Deserialize, Serialize};

use crate::config::{self, Config};
use crate::input_state::InputState;

/// Commands sent from the host to the device.
#[derive(Deserialize)]
#[allow(clippy::large_enum_variant)] // no_std: can't Box
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
#[allow(clippy::large_enum_variant)] // no_std: can't Box
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
    pub num_buttons: u8,
    pub buttons: [bool; config::MAX_DIGITAL_INPUTS],
    pub num_touch_pads: u8,
    pub touch_pads: [bool; config::MAX_DIGITAL_INPUTS],
    pub num_pots: u8,
    pub pots: [u8; config::MAX_ANALOG_INPUTS],
    pub ldr: u8,
    pub accel_x: u8,
    pub accel_y: u8,
    pub accel_tap: bool,
}

#[derive(PartialEq, Eq)]
pub enum Action {
    None,
    Save,
}

pub fn handle_frame(frame: &mut [u8], config: &mut Config, resp: &mut [u8]) -> (usize, Action) {
    let Ok(request) = postcard::from_bytes_cobs::<Request>(frame) else {
        return encode_response(&Response::Error("bad frame"), resp, Action::None);
    };

    match request {
        Request::Ping => encode_response(&Response::Pong, resp, Action::None),
        Request::Version => {
            encode_response(&Response::Version("midictrl 0.1.0"), resp, Action::None)
        }
        Request::GetConfig => encode_response(&Response::Config(*config), resp, Action::None),
        Request::PutConfig(new_config) => {
            if new_config.validate() {
                *config = new_config;
                encode_response(&Response::Ok, resp, Action::None)
            } else {
                encode_response(&Response::Error("invalid pin config"), resp, Action::None)
            }
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
    postcard::to_slice_cobs(&resp, buf).map_or(0, |used| used.len())
}

pub fn encode_error(msg: &str, buf: &mut [u8]) -> usize {
    postcard::to_slice_cobs(&Response::Error(msg), buf).map_or(0, |used| used.len())
}

/// Run the serial command/monitor loop. This is an async function that
/// handles CDC-ACM communication with the host configurator.
pub async fn serial_task(
    serial_class: &mut CdcAcmClass<'static, Driver<'static, USB>>,
    flash: &mut flash::Flash<'static, FLASH, flash::Blocking, { config::FLASH_SIZE }>,
    cfg: &RefCell<Config>,
    input_state: &InputState,
) {
    loop {
        serial_class.wait_connection().await;
        defmt::info!("serial connected");

        let mut frame_buf = [0u8; 2048];
        let mut frame_pos = 0usize;
        let mut last_monitor_send = Instant::now();

        loop {
            // Interleave command processing with monitor snapshots.
            let mut buf = [0u8; 64];

            let read_or_tick = select(
                serial_class.read_packet(&mut buf),
                Timer::after(Duration::from_millis(10)),
            )
            .await;

            match read_or_tick {
                Either::First(result) => {
                    match result {
                        Ok(n) => {
                            for &b in &buf[..n] {
                                if b == 0x00 {
                                    // End of COBS frame
                                    if frame_pos > 0 {
                                        let mut resp = [0u8; 2048];
                                        let (resp_len, action) = handle_frame(
                                            &mut frame_buf[..frame_pos],
                                            &mut cfg.borrow_mut(),
                                            &mut resp,
                                        );
                                        if action == Action::Save {
                                            if config::save_config(flash, &cfg.borrow()) {
                                                send_serial(serial_class, &resp[..resp_len]).await;
                                            } else {
                                                let mut err_resp = [0u8; 64];
                                                let n = encode_error("save failed", &mut err_resp);
                                                if n > 0 {
                                                    send_serial(serial_class, &err_resp[..n]).await;
                                                }
                                            }
                                        } else if resp_len > 0 {
                                            send_serial(serial_class, &resp[..resp_len]).await;
                                        }
                                        frame_pos = 0;
                                    }
                                } else if frame_pos < frame_buf.len() {
                                    frame_buf[frame_pos] = b;
                                    frame_pos += 1;
                                }
                            }
                        }
                        Err(EndpointError::Disabled) => break,
                        Err(EndpointError::BufferOverflow) => {
                            defmt::warn!("serial overflow");
                        }
                    }
                }
                Either::Second(()) => {}
            }

            // Send monitor snapshot at ~50ms intervals
            if Instant::now().duration_since(last_monitor_send).as_millis() >= 50 {
                last_monitor_send = Instant::now();
                let mut resp = [0u8; 256];
                let snapshot = {
                    let cur = cfg.borrow();
                    input_state.snapshot(cur.num_buttons, cur.num_touch_pads, cur.num_pots)
                };
                let n = encode_monitor(snapshot, &mut resp);
                if n > 0 {
                    send_serial(serial_class, &resp[..n]).await;
                }
            }
        }
        defmt::info!("serial disconnected");
    }
}

/// Write a byte slice over CDC-ACM serial in 64-byte chunks.
/// Returns `true` if the entire payload was sent successfully.
async fn send_serial(serial: &mut CdcAcmClass<'static, Driver<'static, USB>>, data: &[u8]) -> bool {
    let mut sent = 0;
    while sent < data.len() {
        let end = (sent + 64).min(data.len());
        if serial.write_packet(&data[sent..end]).await.is_err() {
            return false;
        }
        sent = end;
    }
    // Send ZLP if the payload was a non-zero exact multiple of 64 bytes
    if !data.is_empty() && data.len().is_multiple_of(64) && serial.write_packet(&[]).await.is_err()
    {
        return false;
    }
    true
}
