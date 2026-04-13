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

use crate::config::{self, Config, MAX_LOOP_LAYERS};
#[cfg(target_os = "none")]
use crate::input_state::InputState;
#[cfg(target_os = "none")]
use crate::looper::Looper;

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
    looper: &RefCell<Looper>,
) {
    loop {
        serial.wait_connection().await;
        defmt::info!("serial connected");
        run_session(serial, flash, cfg, input_state, looper).await;
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
    looper: &RefCell<Looper>,
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
                Ok(n) => {
                    process_bytes(&usb_buf[..n], &mut assembler, serial, flash, cfg, looper).await
                }
                Err(EndpointError::Disabled) => return,
                Err(EndpointError::BufferOverflow) => defmt::warn!("serial overflow"),
            }
        }

        if last_monitor.elapsed().as_millis() >= MONITOR_INTERVAL_MS {
            last_monitor = Instant::now();
            send_monitor_snapshot(serial, cfg, input_state, looper).await;
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
    looper: &RefCell<Looper>,
) {
    for &b in bytes {
        if let Some(frame) = assembler.push(b) {
            handle_frame(frame, serial, flash, cfg, looper).await;
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
    looper: &RefCell<Looper>,
) {
    let result = handle_request(frame, &mut cfg.borrow_mut(), &mut looper.borrow_mut());

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
    looper: &RefCell<Looper>,
) {
    let snapshot = {
        let cur = cfg.borrow();
        input_state.snapshot(cur.num_buttons, cur.num_touch_pads, cur.num_pots)
    };
    send_response(serial, &Response::Monitor(snapshot)).await;

    // Send loop state if looper is enabled
    let loop_state = if cfg.borrow().loop_cfg.enabled {
        let lp = looper.borrow();
        let mut layer_states = [0u8; MAX_LOOP_LAYERS];
        let mut layer_event_counts = [0u16; MAX_LOOP_LAYERS];
        let num_layers = lp.num_layers;
        for i in 0..num_layers as usize {
            #[allow(clippy::cast_possible_truncation)]
            let idx = i as u8;
            layer_states[i] = lp.layer_state_byte(idx);
            layer_event_counts[i] = lp.layer_event_count(idx);
        }
        Some(LoopState {
            playing: lp.playing,
            num_layers,
            progress: lp.progress_byte(),
            current_tick: lp.current_tick,
            loop_length_ticks: lp.loop_length_ticks,
            layer_states,
            layer_event_counts,
        })
    } else {
        None
    };

    if let Some(state) = loop_state {
        send_response(serial, &Response::LoopState(state)).await;
    }
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
    /// Start recording on a loop layer (0-3).
    LoopRecord(u8),
    /// Stop recording on a loop layer (transitions to playing).
    LoopStopRecord(u8),
    /// Toggle mute on a loop layer.
    LoopToggleMute(u8),
    /// Clear a single loop layer.
    LoopClear(u8),
    /// Stop all: stop transport and clear all layers.
    LoopStopAll,
    /// Start loop transport (play).
    LoopPlay,
    /// Stop loop transport (pause, keep layer data).
    LoopStop,
}

/// Live looper state snapshot, pushed to the host alongside monitor data.
#[derive(Serialize)]
pub struct LoopState {
    pub playing: bool,
    pub num_layers: u8,
    /// Loop progress 0-255.
    pub progress: u8,
    pub current_tick: u16,
    pub loop_length_ticks: u16,
    /// Per-layer state: 0=Empty, 1=Recording, 2=Playing, 3=Muted.
    pub layer_states: [u8; MAX_LOOP_LAYERS],
    /// Per-layer event count.
    pub layer_event_counts: [u16; MAX_LOOP_LAYERS],
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
    LoopState(LoopState),
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
fn handle_request(
    frame: &mut [u8],
    config: &mut Config,
    looper: &mut crate::looper::Looper,
) -> HandleResult<'static> {
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
        Request::LoopRecord(layer) => {
            if layer < config.loop_cfg.num_layers {
                looper.start_recording(layer);
                HandleResult::Reply(Response::Ok)
            } else {
                HandleResult::Reply(Response::Error("invalid layer"))
            }
        }
        Request::LoopStopRecord(layer) => {
            looper.stop_recording(layer);
            HandleResult::Reply(Response::Ok)
        }
        Request::LoopToggleMute(layer) => {
            looper.toggle_mute(layer);
            HandleResult::Reply(Response::Ok)
        }
        Request::LoopClear(layer) => {
            looper.clear_layer(layer);
            HandleResult::Reply(Response::Ok)
        }
        Request::LoopStopAll => {
            looper.stop_all();
            HandleResult::Reply(Response::Ok)
        }
        Request::LoopPlay => {
            looper.start_transport();
            HandleResult::Reply(Response::Ok)
        }
        Request::LoopStop => {
            looper.stop_transport();
            HandleResult::Reply(Response::Ok)
        }
    }
}

/// Accumulates incoming bytes into complete COBS frames delimited by 0x00.
pub(crate) struct FrameAssembler {
    buf: [u8; 2048],
    pos: usize,
}

impl FrameAssembler {
    pub fn new() -> Self {
        Self {
            buf: [0u8; 2048],
            pos: 0,
        }
    }

    /// Returns `Some(slice)` when a complete frame is ready (the 0x00
    /// delimiter was received), or `None` otherwise.
    pub fn push(&mut self, byte: u8) -> Option<&mut [u8]> {
        if byte == 0x00 {
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
        }
        // Silently drop bytes beyond capacity -- the frame will be malformed
        // and `handle_request` will return an error when decoding fails.
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
