#[cfg(target_os = "none")]
use core::cell::RefCell;

#[cfg(target_os = "none")]
use embassy_futures::select::{select, Either};
#[cfg(target_os = "none")]
use embassy_rp::flash;
#[cfg(target_os = "none")]
use embassy_rp::peripherals::{FLASH, USB};
#[cfg(target_os = "none")]
use embassy_rp::usb::Driver;
#[cfg(target_os = "none")]
use embassy_time::{Duration, Instant, Timer};
#[cfg(target_os = "none")]
use embassy_usb::class::cdc_acm::CdcAcmClass;
#[cfg(target_os = "none")]
use embassy_usb::driver::EndpointError;
use serde::{Deserialize, Serialize};

use crate::config::{self, Config};
#[cfg(target_os = "none")]
use crate::input_state::InputState;

#[cfg(target_os = "none")]
type Serial<'a> = CdcAcmClass<'a, Driver<'a, USB>>;
#[cfg(target_os = "none")]
type Flash<'a> = flash::Flash<'a, FLASH, flash::Blocking, { config::FLASH_SIZE }>;

#[cfg(target_os = "none")]
const MONITOR_INTERVAL_MS: u64 = 50;

/// Run the serial command/monitor loop. Handles CDC-ACM communication with
/// the host configurator: receives request frames, sends responses, and
/// periodically pushes live input-monitor snapshots.
#[cfg(target_os = "none")]
pub async fn serial_task(
    serial: &mut Serial<'static>,
    flash: &mut Flash<'static>,
    cfg: &RefCell<Config>,
    input_state: &InputState,
) {
    loop {
        serial.wait_connection().await;
        defmt::info!("serial connected");
        run_session(serial, flash, cfg, input_state).await;
        defmt::info!("serial disconnected");
    }
}

/// Runs a single connected session until the host disconnects.
#[cfg(target_os = "none")]
async fn run_session(
    serial: &mut Serial<'static>,
    flash: &mut Flash<'static>,
    cfg: &RefCell<Config>,
    input_state: &InputState,
) {
    let mut assembler = FrameAssembler::new();
    let mut last_monitor = Instant::now();

    loop {
        let mut usb_buf = [0u8; 64];
        let event = select(
            serial.read_packet(&mut usb_buf),
            Timer::after(Duration::from_millis(10)),
        )
        .await;

        if let Either::First(result) = event {
            match result {
                Ok(n) => process_bytes(&usb_buf[..n], &mut assembler, serial, flash, cfg).await,
                Err(EndpointError::Disabled) => return,
                Err(EndpointError::BufferOverflow) => defmt::warn!("serial overflow"),
            }
        }

        if last_monitor.elapsed().as_millis() >= MONITOR_INTERVAL_MS {
            last_monitor = Instant::now();
            send_monitor_snapshot(serial, cfg, input_state).await;
        }
    }
}

/// Feed received USB bytes into the frame assembler and handle any complete
/// frames that result.
#[cfg(target_os = "none")]
async fn process_bytes(
    bytes: &[u8],
    assembler: &mut FrameAssembler,
    serial: &mut Serial<'static>,
    flash: &mut Flash<'static>,
    cfg: &RefCell<Config>,
) {
    for &b in bytes {
        if let Some(frame) = assembler.push(b) {
            handle_frame(frame, serial, flash, cfg).await;
        }
    }
}

/// Process a single complete frame: decode the request, act on it, and send
/// the response (including flash persistence when requested).
#[cfg(target_os = "none")]
async fn handle_frame(
    frame: &mut [u8],
    serial: &mut Serial<'static>,
    flash: &mut Flash<'static>,
    cfg: &RefCell<Config>,
) {
    let result = handle_request(frame, &mut cfg.borrow_mut());

    match result {
        HandleResult::Reply(response) => {
            send_response(serial, &response).await;
        }
        HandleResult::ReplyAndSave(response) => {
            if config::save_config(flash, &cfg.borrow()) {
                send_response(serial, &response).await;
            } else {
                send_response(serial, &Response::Error("save failed")).await;
            }
        }
    }
}

/// Capture and send the current input-monitor snapshot.
#[cfg(target_os = "none")]
async fn send_monitor_snapshot(
    serial: &mut Serial<'static>,
    cfg: &RefCell<Config>,
    input_state: &InputState,
) {
    let snapshot = {
        let cur = cfg.borrow();
        input_state.snapshot(cur.num_buttons, cur.num_touch_pads, cur.num_pots)
    };
    send_response(serial, &Response::Monitor(snapshot)).await;
}

/// Write a byte slice over CDC-ACM serial in 64-byte chunks.
#[cfg(target_os = "none")]
async fn send(serial: &mut Serial<'static>, data: &[u8]) -> bool {
    for chunk in data.chunks(64) {
        if serial.write_packet(chunk).await.is_err() {
            return false;
        }
    }
    // Send ZLP if the payload was a non-zero exact multiple of 64 bytes.
    if !data.is_empty() && data.len().is_multiple_of(64) && serial.write_packet(&[]).await.is_err()
    {
        return false;
    }
    true
}

/// Encode a response and send it. Returns `false` on transport failure.
#[cfg(target_os = "none")]
async fn send_response(serial: &mut Serial<'static>, response: &Response<'_>) -> bool {
    let mut buf = [0u8; 2048];
    let n = response.encode(&mut buf);
    if n > 0 {
        send(serial, &buf[..n]).await
    } else {
        false
    }
}

/// Commands sent from the host to the device.
#[derive(Deserialize)]
#[allow(clippy::large_enum_variant)] // no_std: can't Box
pub enum Request {
    Ping,
    Version,
    GetConfig,
    PutConfig(Config),
    Save,
    Reset,
}

/// Responses sent from the device to the host.
#[derive(Serialize)]
#[allow(clippy::large_enum_variant)] // no_std: can't Box
pub enum Response<'a> {
    Pong,
    Version(&'a str),
    Config(Config),
    Ok,
    Error(&'a str),
    Monitor(MonitorSnapshot),
}

impl Response<'_> {
    fn encode(&self, buf: &mut [u8]) -> usize {
        match postcard::to_slice_cobs(self, buf) {
            Ok(used) => used.len(),
            Err(_) => {
                // Last-resort: try to send a minimal error so the host isn't
                // left hanging. If even that fails, return 0.
                postcard::to_slice_cobs(&Response::Error("encode error"), buf)
                    .map_or(0, |used| used.len())
            }
        }
    }
}

#[derive(Serialize)]
pub struct MonitorSnapshot {
    pub num_buttons: u8,
    pub buttons: [bool; config::MAX_DIGITAL_INPUTS],
    pub num_touch_pads: u8,
    pub touch_pads: [bool; config::MAX_DIGITAL_INPUTS],
    pub touch_filtered: [u16; config::MAX_DIGITAL_INPUTS],
    pub touch_baseline: [u16; config::MAX_DIGITAL_INPUTS],
    pub touch_threshold: [u16; config::MAX_DIGITAL_INPUTS],
    pub num_pots: u8,
    pub pots: [u8; config::MAX_ANALOG_INPUTS],
    pub ldr: u8,
    pub accel_x: u8,
    pub accel_y: u8,
    pub accel_tap: bool,
}

#[cfg(target_os = "none")]
enum HandleResult<'a> {
    Reply(Response<'a>),
    ReplyAndSave(Response<'a>),
}

#[cfg(target_os = "none")]
fn handle_request(frame: &mut [u8], config: &mut Config) -> HandleResult<'static> {
    let Ok(request) = postcard::from_bytes_cobs::<Request>(frame) else {
        return HandleResult::Reply(Response::Error("bad frame"));
    };

    match request {
        Request::Ping => HandleResult::Reply(Response::Pong),
        Request::Version => HandleResult::Reply(Response::Version("midictrl 0.1.0")),
        Request::GetConfig => HandleResult::Reply(Response::Config(*config)),
        Request::PutConfig(new) => {
            if new.validate() {
                *config = new;
                HandleResult::Reply(Response::Ok)
            } else {
                HandleResult::Reply(Response::Error("invalid pin config"))
            }
        }
        Request::Save => HandleResult::ReplyAndSave(Response::Ok),
        Request::Reset => {
            *config = Config::default();
            HandleResult::Reply(Response::Ok)
        }
    }
}

/// Accumulates incoming bytes into complete COBS frames delimited by 0x00.
pub(crate) struct FrameAssembler {
    buf: [u8; 2048],
    pos: usize,
    overflow: bool,
}

impl FrameAssembler {
    pub fn new() -> Self {
        Self {
            buf: [0u8; 2048],
            pos: 0,
            overflow: false,
        }
    }

    /// Returns `Some(slice)` when a complete frame is ready (the 0x00
    /// delimiter was received), or `None` otherwise.
    pub fn push(&mut self, byte: u8) -> Option<&mut [u8]> {
        if byte == 0x00 {
            if self.overflow {
                // Discard the entire overflowed frame rather than
                // returning a truncated buffer that would fail to
                // decode and could desynchronise subsequent frames.
                self.overflow = false;
                self.pos = 0;
                return None;
            }
            if self.pos > 0 {
                let len = self.pos;
                self.pos = 0;
                return Some(&mut self.buf[..len]);
            }
            return None;
        }
        if self.pos < self.buf.len() {
            self.buf[self.pos] = byte;
            self.pos += 1;
        } else {
            self.overflow = true;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_assembler() {
        let mut fa = FrameAssembler::new();
        // Non-zero bytes accumulate, no frame yet
        assert!(fa.push(0x01).is_none());
        assert!(fa.push(0x02).is_none());
        assert!(fa.push(0x03).is_none());
        // Zero delimiter returns the accumulated frame
        let frame = fa.push(0x00).unwrap();
        assert_eq!(frame, &[0x01, 0x02, 0x03]);

        // Consecutive delimiters produce no frame
        assert!(fa.push(0x00).is_none());

        // New frame works after reset
        fa.push(0xBB);
        fa.push(0xCC);
        let frame = fa.push(0x00).unwrap();
        assert_eq!(frame, &[0xBB, 0xCC]);
    }
}
